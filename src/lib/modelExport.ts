import type { GaussianTransform } from "../types/pipeline";
import type { ModelFormat, ModelProgress, ModelValidation, PreparedModelExport, SpzAdjustment } from "./modelExportTypes";
import * as backend from "./modelExportBackend";
import { ownedBinaryChunk } from "./binaryChunk";

const cancelled = () => new DOMException("Export cancelled", "AbortError");

/** Owns one reservation/worker, including reservations arriving after cancel. */
export function createModelExport(projectId: string, transform: GaussianTransform, revision: number, format: ModelFormat, onProgress: (event: ModelProgress) => void, reservation?: string) {
  let id: string | null = null;
  let aborted = false, ended = false, executing = false, ready = false, publishing = false;
  let unlisten: (() => void) | undefined;
  let worker: Worker | undefined;
  let rejectWorker: ((error: Error) => void) | undefined;
  let confirmation: { exportId: string; requestId: number; accept: () => void } | undefined;
  let nativeEncodingStarted = false;
  const ensure = () => { if (aborted) throw cancelled(); };
  const cleanup = async () => {
    unlisten?.(); unlisten = undefined;
    worker?.terminate(); worker = undefined;
    confirmation = undefined;
    if (id && !ended) { await backend.cancelModelExport(id); ended = true; }
  };
  const prepared = (async () => {
    try {
      unlisten = await backend.onModelExportProgress(event => {
        if (event.exportId === id && !aborted && !ended && !publishing && !confirmation && (!ready || (nativeEncodingStarted && event.phase === "encoding"))) onProgress({ exportId: event.exportId, phase: event.phase, progress: event.progress });
      });
      ensure();
      id = reservation ?? await backend.beginModelExport(projectId, transform, revision, format);
      ensure();
      const result = await backend.prepareModelExport(id);
      ensure();
      ready = true;
      return result;
    } catch (error) { await cleanup(); throw error; }
  })();

  const convert = (format: Exclude<ModelFormat, "ply">, info: PreparedModelExport) => new Promise<{ validation: ModelValidation; adjustment?: SpzAdjustment }>((resolve, reject) => {
    rejectWorker = reject;
    worker = new Worker(new URL("../workers/gaussianExport.worker.ts", import.meta.url), { type: "module" });
    const current = worker;
    let encodingRequested = false, encodingStarted = false, confirmationRequested = false;
    let approvedAdjustment: SpzAdjustment | undefined;
    const reply = (value: object, transfer: Transferable[] = []) => {
      if (!aborted && !ended && worker === current) current.postMessage({ ...value, exportId: id }, transfer);
    };
    const fail = (error: Error) => {
      current.terminate(); worker = undefined; rejectWorker = undefined; confirmation = undefined; reject(error);
    };
    current.onmessage = event => {
      if (aborted || ended || worker !== current) return;
      const message = event.data;
      if (message.exportId !== id) return;
      if (message.type === "confirm") {
        const adjustment = message.adjustment as SpzAdjustment | undefined;
        if (format !== "spz" || confirmationRequested || encodingRequested || !Number.isSafeInteger(message.requestId) || !adjustment ||
          !Array.isArray(adjustment.center) || adjustment.center.length !== 3 || !adjustment.center.every(Number.isFinite) ||
          !Number.isFinite(adjustment.scale) || adjustment.scale <= 0 || adjustment.scale > 1) {
          fail(new Error("Invalid SPZ confirmation request")); return;
        }
        confirmationRequested = true;
        confirmation = { exportId: id!, requestId: message.requestId, accept: () => {
          approvedAdjustment = { center: [...adjustment.center], scale: adjustment.scale };
          reply({ type: "confirmReply", requestId: message.requestId, accepted: true });
          onProgress({ exportId: id!, phase: "preparing", progress: null });
        } };
        onProgress({ exportId: id!, phase: "awaitingConfirmation", progress: null, confirmationId: message.requestId, adjustment });
      } else if (message.type === "encode") {
        if (encodingRequested || confirmation || (confirmationRequested && !approvedAdjustment)) {
          fail(new Error("Encoder started before confirmation or started twice")); return;
        }
        encodingRequested = true;
        void backend.startModelEncoding(id!, format).then(filename => {
          if (ended || worker !== current) return;
          ensure();
          encodingStarted = true; nativeEncodingStarted = true;
          onProgress({ exportId: id!, phase: "encoding", progress: null });
          reply({ type: "encodeReply", requestId: message.requestId, filename });
        }).catch(error => { if (!aborted && worker === current) fail(error instanceof Error ? error : new Error(String(error))); });
      } else if (message.type === "read") {
        if (message.file === "output" && !encodingStarted) { fail(new Error("Encoder read output before encoding")); return; }
        void backend.readModelExportChunk(id!, message.file, message.offset, message.length).then(bytes => {
          const copy = ownedBinaryChunk(bytes);
          reply({ type: "readReply", requestId: message.requestId, bytes: copy.buffer }, [copy.buffer]);
        }).catch(error => reply({ type: "readReply", requestId: message.requestId, error: String(error) }));
      } else if (message.type === "write") {
        if (!encodingStarted) { fail(new Error("Encoder wrote before encoding was authorized")); return; }
        void backend.writeModelExportChunk(id!, message.offset, new Uint8Array(message.bytes))
          .then(written => reply({ type: "writeReply", requestId: message.requestId, written }))
          .catch(error => reply({ type: "writeReply", requestId: message.requestId, error: String(error) }));
      } else if (message.type === "progress") {
        if (encodingStarted && !confirmation && ["encoding", "validating"].includes(message.phase)) onProgress({ exportId: id!, phase: message.phase, progress: message.progress, detail: message.detail });
      }
      else if (message.type === "done") {
        if (!encodingStarted) { fail(new Error("Encoder finished before encoding was authorized")); return; }
        current.terminate(); worker = undefined; rejectWorker = undefined;
        resolve({ validation: message.validation, ...(approvedAdjustment ? { adjustment: approvedAdjustment } : {}) });
      } else if (message.type === "error") fail(new Error(message.message));
    };
    current.onerror = event => fail(new Error(event.message || "Model encoder worker failed"));
    current.postMessage({ type: "start", exportId: id!, format, sourceSize: info.fileSize, splatCount: info.splatCount, shBands: info.shBands });
  });

  return {
    prepared,
    confirmAdjustment(exportId: string, requestId: number) {
      if (aborted || ended || !confirmation || confirmation.exportId !== exportId || confirmation.requestId !== requestId) return false;
      const accepted = confirmation; confirmation = undefined;
      accepted.accept();
      return true;
    },
    async export() {
      // A duplicate caller must not cancel the already running operation.
      if (executing || ended) throw new Error("Model export already started or finished");
      executing = true;
      try {
        const info = await prepared;
        ensure();
        let encoded: { validation: ModelValidation; adjustment?: SpzAdjustment } | undefined;
        if (format === "ply") {
          nativeEncodingStarted = true;
          onProgress({ exportId: id!, phase: "encoding", progress: 0 });
          await backend.startModelEncoding(id!, format);
        } else {
          onProgress({ exportId: id!, phase: "preparing", progress: null });
          encoded = await convert(format, info);
        }
        ensure();
        publishing = true;
        onProgress({ exportId: id!, phase: "publishing", progress: null });
        const result = await backend.completeModelExport(id!, encoded?.validation ?? null);
        ended = true;
        unlisten?.(); unlisten = undefined;
        return { ...result, ...(encoded?.adjustment ? { coordinateAdjustment: encoded.adjustment } : {}) };
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
