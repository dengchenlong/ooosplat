import {
  ReadStream,
  createChunkDataPool,
  readFile,
  readFileInfo,
  writeSource,
  type FileSystem,
  type InputFormat,
  type ReadFileSystem,
  type ReadSource,
  type Writer,
} from "@playcanvas/splat-transform";

type StartMessage = { type: "start"; importId: string; filename: string; format: InputFormat; size: number; convert: boolean };
type ReadReply = { type: "readReply"; requestId: number; bytes?: ArrayBuffer; error?: string };
type WriteReply = { type: "writeReply"; requestId: number; written?: number; error?: string };
type Incoming = StartMessage | ReadReply | WriteReply | { type: "cancel" };

let cancelled = false;
let sequence = 0;
const pending = new Map<number, { resolve: (value: unknown) => void; reject: (error: Error) => void }>();

const rpc = <T>(type: "read" | "write", value: Record<string, unknown>, transfer: Transferable[] = []) => new Promise<T>((resolve, reject) => {
  const requestId = ++sequence;
  pending.set(requestId, { resolve: resolve as (value: unknown) => void, reject });
  self.postMessage({ type, requestId, ...value }, { transfer });
});

class ImportReadStream extends ReadStream {
  private offset: number;
  private readonly end: number | undefined;
  constructor(private readonly filename: string, start: number, end: number | undefined) {
    super(end == null ? undefined : end - start);
    this.offset = start;
    this.end = end;
  }
  async pull(target: Uint8Array): Promise<number> {
    if (cancelled) throw new Error("cancelled");
    const remaining = this.end == null ? target.byteLength : Math.max(0, this.end - this.offset);
    if (remaining === 0) return 0;
    const length = Math.min(1_024_000, target.byteLength, remaining);
    const bytes = await rpc<ArrayBuffer>("read", { filename: this.filename, offset: this.offset, length });
    const source = new Uint8Array(bytes);
    target.set(source);
    this.offset += source.byteLength;
    this.bytesRead += source.byteLength;
    return source.byteLength;
  }
  close() { /* The backend session owns the file handle lifetime. */ }
}

class ImportReadFileSystem implements ReadFileSystem {
  constructor(private readonly entryName: string, private readonly entrySize: number) {}
  async createSource(filename: string): Promise<ReadSource> {
    const size = filename === this.entryName ? this.entrySize : undefined;
    return { size, seekable: true, read: (start = 0, end = size) => new ImportReadStream(filename, start, end), close() {} };
  }
}

class ImportWriter implements Writer {
  bytesWritten = 0;
  async write(data: Uint8Array) {
    if (cancelled) throw new Error("cancelled");
    const copy = data.slice();
    this.bytesWritten = await rpc<number>("write", { offset: this.bytesWritten, bytes: copy.buffer }, [copy.buffer]);
  }
  close() {}
  abort() { cancelled = true; }
}

const outputFileSystem: FileSystem = {
  createWriter: () => new ImportWriter(),
  mkdir: async () => undefined,
};

const run = async (message: StartMessage) => {
  const fs = new ImportReadFileSystem(message.filename, message.size);
  const options = { filename: message.filename, inputFormat: message.format, fileSystem: fs };
  self.postMessage({ type: "progress", phase: "analysis", progress: 0.05 });
  const info = await readFileInfo(options);
  if (!info.gaussian) throw new Error("所选文件不是可编辑的 3D Gaussian 模型");
  if (info.model === "2dgs") throw new Error("暂不支持 2D Gaussian Splatting 模型");
  if (message.convert) {
    self.postMessage({ type: "progress", phase: "conversion", progress: 0.15 });
    const sources = await readFile(options);
    if (sources.length !== 1) throw new Error("暂不支持多场景或 Streamed SOG 模型");
    try {
      await writeSource({ filename: "model.ply", outputFormat: "ply", source: sources[0], pool: createChunkDataPool(), options: {} }, outputFileSystem);
    } finally {
      sources.forEach(source => source.close());
    }
  }
  self.postMessage({ type: "done", info: { splatCount: info.numGaussians, shBands: info.shBands, model: info.model } });
};

self.onmessage = (event: MessageEvent<Incoming>) => {
  const message = event.data;
  if (message.type === "cancel") { cancelled = true; return; }
  if (message.type === "readReply" || message.type === "writeReply") {
    const waiter = pending.get(message.requestId);
    if (!waiter) return;
    pending.delete(message.requestId);
    if (message.error) waiter.reject(new Error(message.error));
    else waiter.resolve(message.type === "readReply" ? message.bytes! : message.written!);
    return;
  }
  if (message.type === "start") void run(message).catch(error => self.postMessage({ type: "error", message: error instanceof Error ? error.message : String(error) }));
};
