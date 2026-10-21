import { ReadStream, readFile, readFileInfo, writeSource, createChunkDataPool, WorkerQueue, WebPCodec, logger, bakeTransform, Transform, type ReadFileSystem, type ReadSource, type FileSystem, type Writer, type InputFormat } from "@playcanvas/splat-transform";
import webpWasmUrl from "@playcanvas/splat-transform/lib/webp.wasm?url";
import type { ModelWorkerStart, SpzAdjustment } from "../lib/modelExportTypes";
import { validateDecodedLayer } from "../lib/modelExportValidation";
import { createSpzBounds, includeSpzChunk, planSpzAdjustment, assertSpzAdjustmentSizes, adjustedSpzSource } from "../lib/spzAdjustment";
// Already inside a dedicated worker: avoid extra WASM heaps and GPU contention.
WorkerQueue.maxWorkers = 0;
WebPCodec.wasmUrl = webpWasmUrl;
const CHUNK = 1024000;
let sequence = 0, outputSize = 0, started = false;
let exportId = "";
let ioQueue = Promise.resolve();
const pending = new Map<number, {
    resolve: (value: unknown) => void;
    reject: (error: Error) => void;
}>();
const rpc = <T>(type: "read" | "write" | "confirm" | "encode", value: Record<string, unknown>, transfer: Transferable[] = []) => {
    const result = ioQueue.then(() => new Promise<T>((resolve, reject) => {
        const requestId = ++sequence;
        pending.set(requestId, { resolve: resolve as (v: unknown) => void, reject });
        self.postMessage({ type, exportId, requestId, ...value }, { transfer });
    }));
    ioQueue = result.then(() => undefined, () => undefined);
    return result;
};
class Stream extends ReadStream {
    private offset: number;
    constructor(private file: "source" | "output", start: number, private end: number) { super(end - start); this.offset = start; }
    async pull(target: Uint8Array) {
        const length = Math.min(CHUNK, target.length, this.end - this.offset);
        if (length === 0)
            return 0;
        const bytes = new Uint8Array(await rpc<ArrayBuffer>("read", { file: this.file, offset: this.offset, length }));
        if (bytes.length !== length)
            throw new Error("Invalid or truncated export chunk");
        target.set(bytes);
        this.offset += bytes.length;
        this.bytesRead += bytes.length;
        return bytes.length;
    }
    close() { }
}
class FS implements ReadFileSystem {
    constructor(private file: "source" | "output", private size: number) { }
    async createSource(): Promise<ReadSource> { return { size: this.size, seekable: true, read: (start = 0, end = this.size) => new Stream(this.file, start, end), close() { } }; }
}
class Output implements Writer {
    bytesWritten = 0;
    async write(data: Uint8Array) { for (let offset = 0; offset < data.length; offset += CHUNK) {
        const copy = data.slice(offset, offset + CHUNK), expected = this.bytesWritten + copy.length;
        this.bytesWritten = await rpc<number>("write", { offset: this.bytesWritten, bytes: copy.buffer }, [copy.buffer]);
        if (this.bytesWritten !== expected)
            throw new Error("Invalid export write acknowledgement");
        outputSize = this.bytesWritten;
    } }
    close() { }
    abort() { }
}
const outputFS: FileSystem = { createWriter: () => new Output(), mkdir: async () => undefined };
logger.setRenderer({ handle(event) { if (event.kind === "barTick" && event.total > 0)
        self.postMessage({ type: "progress", exportId, phase: "encoding", progress: null, detail: event.name }); } });
async function run(message: ModelWorkerStart) {
    const options = { filename: "edited-view.ply", inputFormat: "ply" as const, fileSystem: new FS("source", message.sourceSize) };
    const info = await readFileInfo(options);
    if (!info.gaussian || info.numGaussians !== message.splatCount || info.shBands !== message.shBands)
        throw new Error("Edited model view validation failed");
    const sources = await readFile(options);
    if (sources.length !== 1)
        throw new Error("Multiple scenes are not supported");
    let encodingSource = sources[0];
    let adjustment: SpzAdjustment | null = null;
    let filename = "";
    const pool = createChunkDataPool({ maxPooledBytes: 0 });
    try {
        if (message.format === "spz") {
            encodingSource = bakeTransform(encodingSource, Transform.PLY);
            const source = encodingSource, meta = source.meta, bounds = createSpzBounds();
            const positionPool = createChunkDataPool({ chunkSize: Math.min(meta.chunkSize, meta.numGaussians), maxPooledBytes: 0 });
            try {
                for (let chunkIndex = 0; chunkIndex < meta.numChunks[0]; chunkIndex++) {
                    const count = Math.min(meta.chunkSize, meta.numGaussians - chunkIndex * meta.chunkSize);
                    const position = positionPool.acquire("position", meta.layouts.position!, count);
                    const geometric = positionPool.acquire("geometric", meta.layouts.geometric!, count);
                    try { await source.read({ chunkIndex, position, geometric }); includeSpzChunk(bounds, position, geometric); }
                    finally { position.release(); geometric.release(); }
                }
            } finally { positionPool.destroy(); }
            if (bounds.count !== message.splatCount) throw new Error("SPZ preflight count mismatch");
            adjustment = planSpzAdjustment(bounds);
            if (adjustment) {
                const accepted = await rpc<boolean>("confirm", { adjustment });
                if (!accepted) throw new Error("SPZ adjustment was not confirmed");
                assertSpzAdjustmentSizes(bounds, adjustment);
                encodingSource = adjustedSpzSource(encodingSource, adjustment);
            }
        }
        // The host creates the output only after preflight and explicit consent.
        filename = await rpc<string>("encode", {});
        await writeSource({ filename, outputFormat: message.format === "compressedPly" ? "compressed-ply" : message.format === "sog" ? "sog-bundle" : "spz", source: encodingSource, pool, options: { iterations: 10, spzVersion: 4, webpEffort: 6 } }, outputFS);
    }
    finally {
        await Promise.all(sources.map(source => source.close()));
        pool.destroy();
    }
    self.postMessage({ type: "progress", exportId, phase: "validating", progress: null });
    const inputFormat: InputFormat = message.format === "compressedPly" ? "ply" : message.format;
    const result = await readFileInfo({ filename, inputFormat, fileSystem: new FS("output", outputSize) });
    if (!result.gaussian || result.numGaussians !== message.splatCount || result.shBands !== message.shBands || result.model !== info.model)
        throw new Error("Export round-trip validation failed: Gaussian count, SH bands or render model changed");
    // SPZ/SOG metadata alone does not validate the payload. Decode every layer,
    // one bounded chunk at a time, before allowing the atomic publication.
    const decoded = await readFile({ filename, inputFormat, fileSystem: new FS("output", outputSize) });
    if (decoded.length !== 1)
        throw new Error("Invalid encoded scene count");
    const source = decoded[0], meta = source.meta;
    const validationPool = createChunkDataPool({ chunkSize: Math.min(meta.chunkSize, meta.numGaussians), maxPooledBytes: 0 });
    try {
        for (let chunkIndex = 0; chunkIndex < meta.numChunks[0]; chunkIndex++) {
            const count = Math.min(meta.chunkSize, meta.numGaussians - chunkIndex * meta.chunkSize);
            const position = validationPool.acquire("position", meta.layouts.position!, count);
            const geometric = validationPool.acquire("geometric", meta.layouts.geometric!, count);
            const color = validationPool.acquire("color", meta.layouts.color!, count);
            try {
                await source.read({ chunkIndex, position, geometric, color });
                for (const data of [position, geometric, color]) {
                    validateDecodedLayer(data);
                }
            }
            finally {
                position.release();
                geometric.release();
                color.release();
            }
        }
    }
    finally {
        await Promise.all(decoded.map(source => source.close()));
        validationPool.destroy();
    }
    self.postMessage({ type: "done", exportId, validation: { splatCount: result.numGaussians, shBands: result.shBands }, adjustment });
}
self.onmessage = (event: MessageEvent) => {
    const m = event.data;
    if (m.type !== "start" && m.exportId !== exportId) return;
    if (m.type === "readReply" || m.type === "writeReply" || m.type === "confirmReply" || m.type === "encodeReply") {
        const p = pending.get(m.requestId);
        if (!p)
            return;
        pending.delete(m.requestId);
        if (m.error)
            p.reject(new Error(m.error));
        else
            p.resolve(m.type === "readReply" ? m.bytes : m.type === "writeReply" ? m.written : m.type === "confirmReply" ? m.accepted : m.filename);
    }
    else if (m.type === "start" && !started) {
        started = true;
        exportId = m.exportId;
        void run(m).catch(error => self.postMessage({ type: "error", exportId, message: String(error) }));
    }
};
