import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { createModelExport } from "./modelExport";
import * as backend from "./modelExportBackend";
import type { GaussianTransform } from "../types/pipeline";

vi.mock("./modelExportBackend", () => ({
  beginModelExport: vi.fn(), prepareModelExport: vi.fn(), startModelEncoding: vi.fn(), completeModelExport: vi.fn(),
  cancelModelExport: vi.fn(), readModelExportChunk: vi.fn(), writeModelExportChunk: vi.fn(), onModelExportProgress: vi.fn(),
}));
class Encoder {
  static instances: Encoder[] = [];
  constructor() { Encoder.instances.push(this); }
  postMessage = vi.fn(); terminate = vi.fn();
  onmessage?: (e: { data: unknown }) => void;
  onerror?: (e: { message: string }) => void;
  emit(data: object) { this.onmessage?.({ data: { exportId: "export-1", ...data } }); }
}
const info = { exportId: "export-1", fileSize: 1000, splatCount: 3, shBands: 3 };
const transform: GaussianTransform = { position: [0, 0, 0], rotation: [0, 0, 0], scale: 1 };
const tick = async () => { await new Promise(resolve => setTimeout(resolve, 0)); };
const beginEncoding = async (worker: Encoder) => { worker.emit({ type: "encode", requestId: 50 }); await tick(); };
const deferred = <T,>() => { let resolve!: (v: T) => void; const promise = new Promise<T>(r => { resolve = r; }); return { promise, resolve }; };
let progress: Parameters<typeof backend.onModelExportProgress>[0];
let unlisten: ReturnType<typeof vi.fn>;
beforeEach(() => {
  vi.resetAllMocks(); Encoder.instances = []; vi.stubGlobal("Worker", Encoder);
  unlisten = vi.fn();
  vi.mocked(backend.onModelExportProgress).mockImplementation(async cb => { progress = cb; return unlisten; });
  vi.mocked(backend.beginModelExport).mockResolvedValue("export-1");
  vi.mocked(backend.prepareModelExport).mockResolvedValue(info);
  vi.mocked(backend.startModelEncoding).mockResolvedValue("edit.spz");
  vi.mocked(backend.completeModelExport).mockResolvedValue({ path: "project/edit.spz", fileSize: 100, splatCount: 3, format: "spz" });
  vi.mocked(backend.cancelModelExport).mockResolvedValue();
});
afterEach(() => vi.unstubAllGlobals());

describe("token-scoped model exports", () => {
  it("adopts an application reservation without starting a second export", async () => {
    const op = createModelExport("p", transform, 7, "ply", vi.fn(), "export-1");
    await op.prepared; await op.export();
    expect(backend.beginModelExport).not.toHaveBeenCalled();
    expect(backend.prepareModelExport).toHaveBeenCalledExactlyOnceWith("export-1");
    expect(backend.completeModelExport).toHaveBeenCalledExactlyOnceWith("export-1", null);
  });
  it("publishes standard PLY without a Worker and ignores unrelated and late progress", async () => {
    const preparation = deferred<typeof info>();
    vi.mocked(backend.prepareModelExport).mockReturnValue(preparation.promise);
    const updates = vi.fn(), op = createModelExport("p", transform, 7, "ply", updates);
    await tick();
    expect(backend.beginModelExport).toHaveBeenCalledWith("p", transform, 7, "ply");
    progress({ exportId: "old", projectId: "p", phase: "preparing", progress: 10 });
    expect(updates).not.toHaveBeenCalled();
    progress({ exportId: "export-1", projectId: "p", phase: "preparing", progress: 30 });
    expect(updates).toHaveBeenCalledWith({ exportId: "export-1", phase: "preparing", progress: 30 });
    preparation.resolve(info); await op.prepared;
    updates.mockClear();
    progress({ exportId: "export-1", projectId: "p", phase: "preparing", progress: 100 });
    expect(updates).not.toHaveBeenCalled();
    await op.export();
    expect(Encoder.instances).toHaveLength(0);
    expect(backend.completeModelExport).toHaveBeenCalledWith("export-1", null);
    updates.mockClear(); progress({ exportId: "export-1", projectId: "p", phase: "preparing", progress: 100 });
    expect(updates).not.toHaveBeenCalled(); expect(unlisten).toHaveBeenCalled();
  });
  it("exchanges raw chunks and publishes only after worker validation", async () => {
    const op = createModelExport("p", transform, 7, "spz", vi.fn()); await op.prepared;
    const result = op.export(); await tick();
    const worker = Encoder.instances[0];
    expect(worker.postMessage).toHaveBeenCalledWith(expect.objectContaining({ format: "spz", sourceSize: 1000, shBands: 3 }));
    expect(backend.startModelEncoding).not.toHaveBeenCalled();
    expect(backend.completeModelExport).not.toHaveBeenCalled();
    vi.mocked(backend.readModelExportChunk).mockResolvedValue(new Uint8Array([1, 2]).buffer);
    worker.emit({ type: "read", requestId: 1, file: "source", offset: 0, length: 2 }); await tick();
    expect(backend.readModelExportChunk).toHaveBeenCalledWith("export-1", "source", 0, 2);
    vi.mocked(backend.writeModelExportChunk).mockResolvedValue(2);
    await beginEncoding(worker);
    worker.emit({ type: "write", requestId: 2, offset: 0, bytes: new Uint8Array([1, 2]).buffer }); await tick();
    expect(backend.writeModelExportChunk).toHaveBeenCalledWith("export-1", 0, new Uint8Array([1, 2]));
    worker.emit({ type: "done", validation: { splatCount: 3, shBands: 3 } }); await result;
    expect(backend.completeModelExport).toHaveBeenCalledWith("export-1", { splatCount: 3, shBands: 3 });
    expect(worker.terminate).toHaveBeenCalled();
  });
  it("does not let delayed native encoding progress replace the publication state", async () => {
    const publication = deferred<Awaited<ReturnType<typeof backend.completeModelExport>>>();
    vi.mocked(backend.completeModelExport).mockReturnValue(publication.promise);
    const updates = vi.fn(), op = createModelExport("p", transform, 7, "ply", updates);
    await op.prepared;
    const result = op.export(); await tick();
    expect(updates).toHaveBeenLastCalledWith({ exportId: "export-1", phase: "publishing", progress: null });
    updates.mockClear();
    progress({ exportId: "export-1", projectId: "p", phase: "encoding", progress: 95 });
    expect(updates).not.toHaveBeenCalled();
    publication.resolve({ path: "project/edit.ply", fileSize: 1000, splatCount: 3, format: "ply" });
    await expect(result).resolves.toMatchObject({ format: "ply" });
  });
  it("terminates on cancel and discards late binary replies", async () => {
    const op = createModelExport("p", transform, 7, "sog", vi.fn()); await op.prepared;
    const result = op.export(); const rejection = expect(result).rejects.toMatchObject({ name: "AbortError" }); await tick();
    const worker = Encoder.instances[0], read = deferred<ArrayBuffer>();
    vi.mocked(backend.readModelExportChunk).mockReturnValue(read.promise);
    worker.emit({ type: "read", requestId: 1, file: "source", offset: 0, length: 2 });
    await op.cancel(); await rejection;
    const calls = worker.postMessage.mock.calls.length;
    read.resolve(new ArrayBuffer(2)); await tick();
    expect(worker.postMessage).toHaveBeenCalledTimes(calls);
    expect(backend.completeModelExport).not.toHaveBeenCalled(); expect(backend.cancelModelExport).toHaveBeenCalledWith("export-1");
  });
  it("cancels a late reservation instead of leaking its lock", async () => {
    const reservation = deferred<string>(); vi.mocked(backend.beginModelExport).mockReturnValue(reservation.promise);
    const op = createModelExport("p", transform, 7, "ply", vi.fn()); const rejection = expect(op.prepared).rejects.toMatchObject({ name: "AbortError" });
    await tick(); await op.cancel(); reservation.resolve("late"); await rejection;
    expect(backend.prepareModelExport).not.toHaveBeenCalled(); expect(backend.cancelModelExport).toHaveBeenCalledWith("late");
  });
  it.each(["compressedPly", "spz", "sog"] as const)("starts %s once for a large model without an estimated-memory gate", async format => {
    const large = { ...info, fileSize: 3 * 1024 ** 3, splatCount: 5_000_000 };
    vi.mocked(backend.prepareModelExport).mockResolvedValue(large);
    const op = createModelExport("p", transform, 7, format, vi.fn()); await op.prepared;
    const result = op.export(); await tick();
    expect(backend.startModelEncoding).not.toHaveBeenCalled();
    expect(Encoder.instances).toHaveLength(1);
    expect(Encoder.instances[0].postMessage).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ format, sourceSize: large.fileSize, splatCount: large.splatCount }));
    await beginEncoding(Encoder.instances[0]);
    expect(backend.startModelEncoding).toHaveBeenCalledExactlyOnceWith("export-1", format);
    await expect(op.export()).rejects.toThrow("already started");
    Encoder.instances[0].emit({ type: "done", validation: { splatCount: large.splatCount, shBands: 3 } });
    await result;
    expect(backend.completeModelExport).toHaveBeenCalledExactlyOnceWith("export-1", { splatCount: large.splatCount, shBands: 3 });
  });
  it("cleans up encoder failure, permits a new session, and rejects duplicate starts without cancelling the first", async () => {
    const op = createModelExport("p", transform, 7, "compressedPly", vi.fn()); await op.prepared;
    const first = op.export(); await tick();
    await expect(op.export()).rejects.toThrow("already started");
    expect(backend.cancelModelExport).not.toHaveBeenCalled();
    Encoder.instances[0].emit({ type: "error", message: "disk full" }); await expect(first).rejects.toThrow("disk full");
    expect(backend.cancelModelExport).toHaveBeenCalledWith("export-1");
    const retry = createModelExport("p", transform, 7, "ply", vi.fn()); await retry.prepared; await retry.export();
    expect(backend.completeModelExport).toHaveBeenCalledOnce();
  });

  it("waits for session-bound consent, ignores stale progress and reports only the approved adjustment", async () => {
    const updates = vi.fn(), adjustment = { center: [10000, 2, 3], scale: 1 };
    const op = createModelExport("p", transform, 7, "spz", updates); await op.prepared;
    const result = op.export(); await tick(); const worker = Encoder.instances[0];
    worker.emit({ type: "confirm", requestId: 70, adjustment });
    expect(updates).toHaveBeenLastCalledWith({ exportId: "export-1", phase: "awaitingConfirmation", progress: null, confirmationId: 70, adjustment });
    updates.mockClear();
    progress({ exportId: "export-1", projectId: "p", phase: "encoding", progress: 70 });
    worker.emit({ type: "progress", phase: "encoding", progress: 75 });
    expect(updates).not.toHaveBeenCalled();
    expect(backend.startModelEncoding).not.toHaveBeenCalled(); expect(backend.writeModelExportChunk).not.toHaveBeenCalled();
    expect(op.confirmAdjustment("old", 70)).toBe(false); expect(op.confirmAdjustment("export-1", 71)).toBe(false);
    expect(op.confirmAdjustment("export-1", 70)).toBe(true); expect(op.confirmAdjustment("export-1", 70)).toBe(false);
    expect(worker.postMessage).toHaveBeenLastCalledWith({ type: "confirmReply", exportId: "export-1", requestId: 70, accepted: true }, []);
    expect(backend.startModelEncoding).not.toHaveBeenCalled();
    await beginEncoding(worker);
    expect(backend.startModelEncoding).toHaveBeenCalledExactlyOnceWith("export-1", "spz");
    worker.emit({ type: "done", validation: { splatCount: 3, shBands: 3 }, adjustment });
    await expect(result).resolves.toMatchObject({ coordinateAdjustment: adjustment });
    expect(backend.completeModelExport).toHaveBeenCalledWith("export-1", { splatCount: 3, shBands: 3 });
  });
  it("cancels waiting confirmation without encoding, and requires new consent after retry", async () => {
    const adjustment = { center: [10000, 0, 0], scale: 1 };
    const op = createModelExport("p", transform, 7, "spz", vi.fn()); await op.prepared;
    const result = op.export(), rejection = expect(result).rejects.toMatchObject({ name: "AbortError" });
    await tick(); const old = Encoder.instances[0]; old.emit({ type: "confirm", requestId: 7, adjustment });
    await op.cancel(); await rejection;
    expect(op.confirmAdjustment("export-1", 7)).toBe(false);
    old.emit({ type: "encode", requestId: 8 });
    expect(backend.startModelEncoding).not.toHaveBeenCalled(); expect(backend.completeModelExport).not.toHaveBeenCalled();
    const retry = createModelExport("p", transform, 7, "spz", vi.fn()); await retry.prepared;
    const retried = retry.export(), cancelledAgain = expect(retried).rejects.toMatchObject({ name: "AbortError" });
    await tick(); Encoder.instances[1].emit({ type: "confirm", requestId: 9, adjustment });
    expect(backend.startModelEncoding).not.toHaveBeenCalled();
    await retry.cancel(); await cancelledAgain;
  });
  it("rejects premature writes without creating output", async () => {
    const op = createModelExport("p", transform, 7, "spz", vi.fn()); await op.prepared;
    const result = op.export(); const rejection = expect(result).rejects.toThrow("before encoding was authorized"); await tick();
    Encoder.instances[0].emit({ type: "write", requestId: 8, offset: 0, bytes: new ArrayBuffer(1) }); await rejection;
    expect(backend.startModelEncoding).not.toHaveBeenCalled(); expect(backend.writeModelExportChunk).not.toHaveBeenCalled();
  });
  it.each(["revision changed", "source changed"])("surfaces %s after confirmation without writing or publishing", async reason => {
    vi.mocked(backend.startModelEncoding).mockRejectedValue(new Error(reason));
    const op = createModelExport("p", transform, 7, "spz", vi.fn()); await op.prepared;
    const result = op.export(), rejection = expect(result).rejects.toThrow(reason); await tick();
    const worker = Encoder.instances[0]; worker.emit({ type: "confirm", requestId: 7, adjustment: { center: [10000, 0, 0], scale: 1 } });
    op.confirmAdjustment("export-1", 7); await beginEncoding(worker); await rejection;
    expect(backend.writeModelExportChunk).not.toHaveBeenCalled(); expect(backend.completeModelExport).not.toHaveBeenCalled();
  });
  it("ignores a late encoding reservation after cancellation", async () => {
    const encoding = deferred<string>(); vi.mocked(backend.startModelEncoding).mockReturnValue(encoding.promise);
    const updates = vi.fn(), op = createModelExport("p", transform, 7, "sog", updates); await op.prepared;
    const result = op.export(), rejection = expect(result).rejects.toMatchObject({ name: "AbortError" }); await tick();
    const worker = Encoder.instances[0]; await beginEncoding(worker);
    await op.cancel(); await rejection; const count = worker.postMessage.mock.calls.length; updates.mockClear();
    encoding.resolve("edit.sog"); await tick();
    expect(updates).not.toHaveBeenCalled(); expect(worker.postMessage).toHaveBeenCalledTimes(count);
    expect(backend.completeModelExport).not.toHaveBeenCalled();
  });
});
