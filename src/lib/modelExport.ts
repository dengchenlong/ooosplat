import type { GaussianTransform } from "../types/pipeline";
import type { ModelFormat, ModelProgress, ModelValidation, PreparedModelExport } from "./modelExportTypes";
import * as backend from "./modelExportBackend";
import { ownedBinaryChunk } from "./binaryChunk";

const cancelled = () => new DOMException("Export cancelled", "AbortError");

/** Owns one reservation/worker, including reservations arriving after cancel. */
export function createModelExport(projectId: string, transform: GaussianTransform, revision: number, format: ModelFormat, onProgress: (event: ModelProgress) => void) {
  let id: string | null = null;
  let aborted = false, ended = false, executing = false, ready = false, publishing = false;
  let unlisten: (() => void) | undefined;
  let worker: Worker | undefined;
  let rejectWorker: ((error: Error) => void) | undefined;
  const ensure = () => { if (aborted) throw cancelled(); };
  const cleanup = async () => {
    unlisten?.(); unlisten = undefined;
    worker?.terminate(); worker = undefined;
    if (id && !ended) { await backend.cancelModelExport(id); ended = true; }
  };
  const prepared = (async () => {
    try {
      unlisten = await backend.onModelExportProgress(event => {
        if (event.exportId === id && !aborted && !ended && !publishing && (event.phase === "encoding" || !ready)) onProgress({ exportId: event.exportId, phase: event.phase, progress: event.progress });
      });
      ensure();
      id = await backend.beginModelExport(projectId, transform, revision, format);
      ensure();
      const result = await backend.prepareModelExport(id);
      ensure();
      ready = true;
      return result;
    } catch (error) { await cleanup(); throw error; }
  })();

  const convert = (format: Exclude<ModelFormat, "ply">, filename: string, info: PreparedModelExport) => new Promise<ModelValidation>((resolve, reject) => {
    rejectWorker = reject;
    worker = new Worker(new URL("../workers/gaussianExport.worker.ts", import.meta.url), { type: "module" });
    const current = worker;
    const reply = (value: unknown, transfer: Transferable[] = []) => {
      if (!aborted && !ended && worker === current) current.postMessage(value, transfer);
    };
    const fail = (error: Error) => {
      current.terminate(); worker = undefined; rejectWorker = undefined; reject(error);
    };
    current.onmessage = event => {
      if (aborted || ended || worker !== current) return;
      const message = event.data;
      if (message.exportId !== id) return;
      if (message.type === "read") {
        void backend.readModelExportChunk(id!, message.file, message.offset, message.length).then(bytes => {
          const copy = ownedBinaryChunk(bytes);
          reply({ type: "readReply", requestId: message.requestId, bytes: copy.buffer }, [copy.buffer]);
        }).catch(error => reply({ type: "readReply", requestId: message.requestId, error: String(error) }));
      } else if (message.type === "write") {
        void backend.writeModelExportChunk(id!, message.offset, new Uint8Array(message.bytes))
          .then(written => reply({ type: "writeReply", requestId: message.requestId, written }))
          .catch(error => reply({ type: "writeReply", requestId: message.requestId, error: String(error) }));
      } else if (message.type === "progress") onProgress({ exportId: id!, phase: message.phase, progress: message.progress, detail: message.detail });
      else if (message.type === "done") {
        current.terminate(); worker = undefined; rejectWorker = undefined; resolve(message.validation);
      } else if (message.type === "error") fail(new Error(message.message));
    };
    current.onerror = event => fail(new Error(event.message || "Model encoder worker failed"));
    current.postMessage({ type: "start", exportId: id!, format, filename, sourceSize: info.fileSize, splatCount: info.splatCount, shBands: info.shBands });
  });

  return {
    prepared,
    async export() {
      // A duplicate caller must not cancel the already running operation.
      if (executing || ended) throw new Error("Model export already started or finished");
      executing = true;
      try {
        const info = await prepared;
        ensure();
        if (format !== "ply" && !info.compressionAllowed) throw new Error("Compression exceeds the safe memory budget; choose standard PLY");
        onProgress({ exportId: id!, phase: "encoding", progress: format === "ply" ? 0 : null });
        const filename = await backend.startModelEncoding(id!, format);
        ensure();
        const validation = format === "ply" ? null : await convert(format, filename, info);
        ensure();
        publishing = true;
        onProgress({ exportId: id!, phase: "publishing", progress: null });
        const result = await backend.completeModelExport(id!, validation);
        ended = true;
        unlisten?.(); unlisten = undefined;
        return result;
      } catch (error) { await cleanup(); throw error; }
    },
    async cancel() {
      if (ended) return;
      aborted = true;
      worker?.terminate(); worker = undefined;
      rejectWorker?.(cancelled()); rejectWorker = undefined;
      await cleanup();
    },
  };
}
export type ModelExportOperation = ReturnType<typeof createModelExport>;
