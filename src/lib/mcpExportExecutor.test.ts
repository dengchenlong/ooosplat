// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { createModelExport } from "./modelExport";
import { startMcpExportExecutor, useMcpExportActivity } from "./mcpExportExecutor";
import { useGaussianTransformStore } from "../stores/gaussianTransformStore";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("./modelExport", () => ({ createModelExport: vi.fn() }));
const tick = async () => { await new Promise(resolve => setTimeout(resolve, 0)); };
const listeners = new Map<string, (event: { payload: any }) => void>();
let dispose: (() => Promise<void>) | undefined;
let resolveExport: (value: any) => void;
let rejectExport: (reason: any) => void;
let operation: { export: ReturnType<typeof vi.fn>; cancel: ReturnType<typeof vi.fn>; confirmAdjustment: ReturnType<typeof vi.fn>; prepared: Promise<any> };
const transform = { position: [0, 0, 0], rotation: [0, 0, 0], scale: 1 };
beforeEach(() => {
  vi.resetAllMocks(); listeners.clear();
  useMcpExportActivity.setState({ projectId: null, editorProjectId: null, editorBusy: false });
  useGaussianTransformStore.setState({ descriptor: null, saveState: "saved", transactionStart: null, cropTransactionStart: undefined });
  vi.mocked(invoke).mockResolvedValue(undefined);
  vi.mocked(listen).mockImplementation(async (name, callback) => {
    listeners.set(name, callback as any); return () => { listeners.delete(name); };
  });
  const result = new Promise(resolve => { resolveExport = resolve; });
  const failure = new Promise((_, reject) => { rejectExport = reject; });
  operation = { export: vi.fn(() => Promise.race([result, failure])), cancel: vi.fn().mockResolvedValue(undefined), confirmAdjustment: vi.fn().mockReturnValue(true), prepared: Promise.resolve({}) };
  vi.mocked(createModelExport).mockReturnValue(operation as any);
});
afterEach(async () => { await dispose?.(); dispose = undefined; });
async function setup() {
  const once = startMcpExportExecutor(); expect(startMcpExportExecutor()).toBe(once);
  dispose = await once;
  const executorId = (vi.mocked(invoke).mock.calls.find(([name]) => name === "mcp_export_executor_ready")![1] as Record<string, unknown>).executorId;
  return { exportId: "job", executorId, sessionId: "token", projectId: "project", format: "spz", transform, editRevision: 7 };
}
it("exports saved edits without preview and does not invent frontend completion", async () => {
  const work = await setup(); listeners.get("mcp-model-export-work")!({ payload: work }); await tick();
  expect(createModelExport).toHaveBeenCalledExactlyOnceWith("project", transform, 7, "spz", expect.any(Function), "token");
  expect(useMcpExportActivity.getState().projectId).toBe("project");
  listeners.get("mcp-model-export-work")!({ payload: work });
  listeners.get("mcp-model-export-work")!({ payload: { ...work, executorId: "stale" } });
  expect(createModelExport).toHaveBeenCalledTimes(1);
  resolveExport({}); await tick();
  expect(useMcpExportActivity.getState().projectId).toBeNull();
  expect(vi.mocked(invoke).mock.calls.some(([, args]) => (args as any)?.update?.phase === "completed")).toBe(false);
});
it.each(["unsaved", "editing"])("refuses %s target without saving or encoding", async state => {
  const work = await setup();
  if (state === "unsaved") useGaussianTransformStore.setState({ descriptor: { projectId: "project" } as any, saveState: "dirty" });
  else useMcpExportActivity.setState({ editorProjectId: "project", editorBusy: true });
  listeners.get("mcp-model-export-work")!({ payload: work }); await tick();
  expect(createModelExport).not.toHaveBeenCalled();
  expect(invoke).toHaveBeenCalledWith("update_mcp_model_export", expect.objectContaining({ update: expect.objectContaining({ phase: "failed", errorCode: "EDITS_NOT_SAVED" }) }));
});
it("waits for exact export/executor consent and ignores unrelated controls", async () => {
  const work = await setup(); listeners.get("mcp-model-export-work")!({ payload: work });
  const progress = vi.mocked(createModelExport).mock.calls[0][4];
  progress({ exportId: "token", phase: "awaitingConfirmation", progress: null, confirmationId: 8, adjustment: { center: [10000, 0, 0], scale: 0.5 } }); await tick();
  expect(operation.confirmAdjustment).not.toHaveBeenCalled();
  listeners.get("mcp-model-export-control")!({ payload: { ...work, exportId: "other", action: "confirm", confirmationId: 8 } });
  expect(operation.confirmAdjustment).not.toHaveBeenCalled();
  listeners.get("mcp-model-export-control")!({ payload: { ...work, action: "confirm", confirmationId: 8 } });
  expect(operation.confirmAdjustment).toHaveBeenCalledExactlyOnceWith("token", 8);
});
it("cancels on rejection or page unload, removes listeners and releases activity", async () => {
  const work = await setup(); listeners.get("mcp-model-export-work")!({ payload: work });
  listeners.get("mcp-model-export-control")!({ payload: { ...work, action: "cancel" } }); await tick();
  expect(operation.cancel).toHaveBeenCalled();
  rejectExport(new DOMException("Cancelled", "AbortError")); await tick();
  expect(vi.mocked(invoke).mock.calls.some(([, args]) => (args as any)?.update?.phase === "failed")).toBe(false);
  window.dispatchEvent(new Event("pagehide")); await tick();
  expect(listeners.size).toBe(0); expect(useMcpExportActivity.getState().projectId).toBeNull();
  expect(invoke).toHaveBeenCalledWith("mcp_export_executor_stopped", { executorId: work.executorId });
});
