import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import { createModelExport, type ModelExportOperation } from "./modelExport";
import type { ModelFormat, ModelProgress } from "./modelExportTypes";
import type { GaussianTransform } from "../types/pipeline";
import { useGaussianTransformStore } from "../stores/gaussianTransformStore";

// Shared primitive subscriptions keep progress updates out of the viewer's render path.
export const useMcpExportActivity = create<{
  projectId: string | null; editorProjectId: string | null; editorBusy: boolean;
}>(() => ({ projectId: null, editorProjectId: null, editorBusy: false }));

interface Work {
  exportId: string; executorId: string; sessionId: string; projectId: string;
  format: ModelFormat; transform: GaussianTransform; editRevision: number;
}
interface Control { exportId: string; executorId: string; action: "confirm" | "cancel"; confirmationId?: number }
let singleton: Promise<() => Promise<void>> | undefined;

/** One application-level host, independent of preview mounting or MCP connections. */
export function startMcpExportExecutor() {
  return singleton ??= install().catch(error => { singleton = undefined; throw error; });
}
async function install(): Promise<() => Promise<void>> {
  const executorId = crypto.randomUUID();
  let disposed = false;
  let current: { work: Work; operation: ModelExportOperation } | undefined;
  let updates = Promise.resolve();
  const report = (work: Work, data: object) => {
    // Keep lifecycle/confirmation updates ahead of subsequent progress messages.
    updates = updates.then(() => invoke<void>("update_mcp_model_export", {
      update: { exportId: work.exportId, executorId, ...data },
    }));
    // A rejected report invalidates the execution instead of silently losing consent.
    void updates.catch(async () => {
      if (current?.work.exportId === work.exportId) {
        await current.operation.cancel().catch(() => undefined);
        await invoke("mcp_export_executor_stopped", { executorId }).catch(() => undefined);
        release(work);
      }
    });
  };
  const release = (work: Work) => {
    if (current?.work.exportId !== work.exportId) return;
    current = undefined;
    useMcpExportActivity.setState({ projectId: null });
  };
  const progress = (work: Work, event: ModelProgress) => {
    if (disposed || current?.work.exportId !== work.exportId) return;
    if (event.phase === "awaitingConfirmation") {
      report(work, { phase: "awaiting_confirmation", progress: null, confirmationId: event.confirmationId, adjustment: event.adjustment });
    } else report(work, { phase: event.phase, progress: event.progress });
  };
  const unlistenWork = await listen<Work>("mcp-model-export-work", event => {
    const work = event.payload;
    if (disposed || work.executorId !== executorId || current?.work.exportId === work.exportId) return;
    const editor = useGaussianTransformStore.getState();
    const activity = useMcpExportActivity.getState();
    const targetOpen = editor.descriptor?.projectId === work.projectId;
    const unsaved = targetOpen && (editor.saveState !== "saved" || editor.transactionStart !== null || editor.cropTransactionStart !== undefined);
    const editing = activity.editorProjectId === work.projectId && activity.editorBusy;
    if (current || unsaved || editing) {
      report(work, { phase: "failed", progress: null, errorCode: current ? "EXPORT_BUSY" : "EDITS_NOT_SAVED", error: "Save the project edits and finish the current operation before exporting." });
      return;
    }
    // The backend reserved this exact saved revision and owns the source/output paths.
    updates = Promise.resolve();
    useMcpExportActivity.setState({ projectId: work.projectId });
    const operation = createModelExport(work.projectId, work.transform, work.editRevision, work.format, update => progress(work, update), work.sessionId);
    current = { work, operation };
    report(work, { phase: "preparing", progress: null });
    void operation.export().then(() => {
      // Only complete_gaussian_model_export can publish/mark the job completed.
      release(work);
    }).catch(error => {
      if (!disposed && current?.work.exportId === work.exportId && !(error instanceof DOMException && error.name === "AbortError")) {
        report(work, { phase: "failed", progress: null, error: String(error) });
      }
      release(work);
    });
  });
  let unlistenControl: (() => void) | undefined;
  try {
    unlistenControl = await listen<Control>("mcp-model-export-control", event => {
      const message = event.payload;
      if (disposed || message.executorId !== executorId || current?.work.exportId !== message.exportId) return;
      const { work, operation } = current;
      if (message.action === "cancel") {
        void operation.cancel().catch(() => undefined).finally(() => release(work));
      } else if (message.confirmationId == null || !operation.confirmAdjustment(work.sessionId, message.confirmationId)) {
        report(work, { phase: "failed", progress: null, errorCode: "STALE_CONFIRMATION", error: "SPZ confirmation no longer matches this export." });
        void operation.cancel().catch(() => undefined).finally(() => release(work));
      }
    });
    await invoke("mcp_export_executor_ready", { executorId });
  } catch (error) {
    unlistenWork(); unlistenControl?.(); throw error;
  }
  const dispose = async () => {
    if (disposed) return;
    disposed = true; unlistenWork(); unlistenControl?.();
    window.removeEventListener("pagehide", pageHide);
    await current?.operation.cancel().catch(() => undefined);
    current = undefined; useMcpExportActivity.setState({ projectId: null });
    await invoke("mcp_export_executor_stopped", { executorId }).catch(() => undefined);
    singleton = undefined;
  };
  const pageHide = () => { void dispose(); };
  window.addEventListener("pagehide", pageHide);
  return dispose;
}
