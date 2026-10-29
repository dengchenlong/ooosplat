import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { IDENTITY_TRANSFORM, useGaussianTransformStore } from "../../stores/gaussianTransformStore";
import { PreviewEditPersistence } from "./PreviewEditPersistence";
import { beginGaussianEditSave, commitGaussianEditSave, saveGaussianTransform } from "../../lib/backend";

vi.mock("../../lib/backend", () => ({ beginGaussianEditSave: vi.fn(), commitGaussianEditSave: vi.fn(), saveGaussianTransform: vi.fn() }));
const state = () => useGaussianTransformStore.getState();
const load = (projectId = "project") => state().load({ projectId, modelPath: "final.ply", assetPath: "preview.ply", assetUrl: "asset://preview", editMaskAssetUrl: null, editMaskAssetPath: null, format: "ply", fileSize: 1, splatCount: 8, transform: IDENTITY_TRANSFORM, editing: { crop: null, revision: 0, sourceSplatCount: 8, deletedCount: 0 } });

describe("preview edit persistence", () => {
  let writer: PreviewEditPersistence;
  beforeEach(() => {
    vi.resetAllMocks(); state().close(); load(); writer = new PreviewEditPersistence("project");
    vi.mocked(saveGaussianTransform).mockImplementation(async (_id, transform) => transform);
    vi.mocked(beginGaussianEditSave).mockResolvedValue({ editId: "save", expectedMaskBytes: 1 });
    vi.mocked(commitGaussianEditSave).mockResolvedValue({ crop: null, revision: 1, deletedCount: 1, sourceSplatCount: 8 });
  });
  afterEach(() => { writer.destroy(); state().close(); });

  it("enqueues committed input synchronously, flushes without React frames, and does not write a PLY", async () => {
    state().beginTransaction(); state().setTransformLive({ ...IDENTITY_TRANSFORM, scale: 2 }); state().commitTransaction();
    await writer.flush();
    expect(saveGaussianTransform).toHaveBeenCalledWith("project", { ...IDENTITY_TRANSFORM, scale: 2 });
    expect(beginGaussianEditSave).not.toHaveBeenCalled();
    expect(state().saveState).toBe("saved");
    state().beginTransaction(); state().setTransformLive(state().transform); state().commitTransaction();
    await writer.flush();
    expect(state().saveState).toBe("saved");
    expect(saveGaussianTransform).toHaveBeenCalledTimes(1);
    state().setSelectionMask(new Uint8Array([2])); state().deleteSelection();
    await writer.flush();
    expect(commitGaussianEditSave).toHaveBeenCalledWith("save", new Uint8Array([2]));
    expect(state().editing.revision).toBe(1);
  });

  it("retains current deletions on failure and retries them rather than reading an old mask", async () => {
    vi.mocked(commitGaussianEditSave).mockRejectedValueOnce(new Error("disk unavailable"));
    state().setSelectionMask(new Uint8Array([4])); state().deleteSelection();
    await expect(writer.flush()).rejects.toThrow("disk unavailable");
    expect(state().deletedMask).toEqual(new Uint8Array([4]));
    expect(state().saveState).toBe("error");
    await writer.flush(true);
    expect(commitGaussianEditSave).toHaveBeenLastCalledWith("save", new Uint8Array([4]));
    expect(state().saveState).toBe("saved");
    expect(state().deletedMask).toEqual(new Uint8Array([4]));
  });

  it("serializes revisions and does not let an old writer update another project", async () => {
    let finish!: () => void;
    vi.mocked(saveGaussianTransform).mockImplementationOnce(async (_id, transform) => { await new Promise<void>(resolve => { finish = resolve; }); return transform; });
    state().beginTransaction(); state().setTransformLive({ ...IDENTITY_TRANSFORM, scale: 2 }); state().commitTransaction();
    await Promise.resolve();
    state().beginTransaction(); state().setTransformLive({ ...IDENTITY_TRANSFORM, scale: 3 }); state().commitTransaction();
    const wait = writer.flush(); finish(); await wait;
    expect(vi.mocked(saveGaussianTransform).mock.calls.map(call => call[1].scale)).toEqual([2, 3]);
    writer.destroy(); load("another");
    state().beginTransaction(); state().setTransformLive({ ...IDENTITY_TRANSFORM, scale: 4 }); state().commitTransaction();
    expect(saveGaussianTransform).toHaveBeenCalledTimes(2);
    expect(state().saveState).toBe("dirty");
  });
});
