import type { GaussianImportSession, ImportedGaussianFormat } from "../types/pipeline";
import { readGaussianImportChunk, writeGaussianImportChunk } from "./backend";
import { ownedBinaryChunk } from "./binaryChunk";

export interface GaussianConversionResult { splatCount: number; shBands: number; model: string | null; }
export interface GaussianConversionProgress { phase: "analysis" | "conversion"; progress: number; }

const inputFormat = (format: ImportedGaussianFormat): "ply" | "sog" | "spz" | "splat" | "ksplat" => format === "compressedPly" ? "ply" : format;

export function convertGaussianImport(session: GaussianImportSession, onProgress: (progress: GaussianConversionProgress) => void) {
  const worker = new Worker(new URL("../workers/gaussianImport.worker.ts", import.meta.url), { type: "module" });
  const result = new Promise<GaussianConversionResult>((resolve, reject) => {
    worker.onmessage = event => {
      const message = event.data as { type: string; requestId?: number; filename?: string; offset?: number; length?: number; bytes?: ArrayBuffer; progress?: number; phase?: GaussianConversionProgress["phase"]; written?: number; info?: GaussianConversionResult; message?: string };
      if (message.type === "read") {
        void readGaussianImportChunk(session.importId, message.filename ?? session.sourceName, message.offset ?? 0, message.length ?? 1_024_000)
          .then(bytes => {
            const copy = ownedBinaryChunk(bytes);
            worker.postMessage({ type: "readReply", requestId: message.requestId, bytes: copy.buffer }, [copy.buffer]);
          })
          .catch(error => worker.postMessage({ type: "readReply", requestId: message.requestId, error: String(error) }));
      } else if (message.type === "write") {
        const bytes = new Uint8Array(message.bytes!);
        void writeGaussianImportChunk(session.importId, message.offset ?? 0, bytes)
          .then(written => {
            const estimatedOutputSize = Math.max(1, session.sourceTotalSize * 1.5);
            onProgress({ phase: "conversion", progress: 0.15 + Math.min(0.7, (written / estimatedOutputSize) * 0.7) });
            worker.postMessage({ type: "writeReply", requestId: message.requestId, written });
          })
          .catch(error => worker.postMessage({ type: "writeReply", requestId: message.requestId, error: String(error) }));
      } else if (message.type === "progress") onProgress({ phase: message.phase!, progress: message.progress! });
      else if (message.type === "done") { worker.terminate(); resolve(message.info!); }
      else if (message.type === "error") { worker.terminate(); reject(new Error(message.message)); }
    };
    worker.onerror = error => { worker.terminate(); reject(error); };
    worker.postMessage({ type: "start", importId: session.importId, filename: session.sourceName, format: inputFormat(session.sourceFormat), size: session.sourceSize, convert: session.needsConversion });
  });
  return { result, cancel: () => { worker.postMessage({ type: "cancel" }); worker.terminate(); } };
}
