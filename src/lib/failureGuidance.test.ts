import { describe, expect, it } from "vitest";
import { failureCopy, failureGuidanceKind } from "./failureGuidance";
import { errorSummaryKey } from "./errorSummary";
import { pipelineCommandError, taskFailureContext, type PipelineFailureKind } from "./pipelineError";

describe("versioned failure guidance", () => {
  const mixed = "wgpu error: Out of Memory\nFailed to read counts\nAn IO error happened\nfailed to load dataset";
  it.each<PipelineFailureKind>(["brush_out_of_memory", "brush_resource_limit", "colmap_crash", "mapper_storage", "brush_dataset"])("uses backend %s consistently instead of re-reading the tail", kind => {
    const command = pipelineCommandError({ code: "pipeline_failed", message: mixed, failureKind: kind, classificationVersion: 1 });
    expect(failureGuidanceKind(command)).toBe(kind);
    const context = taskFailureContext({ classification_version: 1, classification: kind });
    expect(errorSummaryKey(mixed, context)).toBe(failureCopy(kind, 1).title);
    expect(failureCopy(kind, 1).tips).toEqual([]);
  });
  it("never falls back to material/GPU guesses for new unknown failures", () => {
    for (const failedStage of ["reconstructing", "trainingSplats"]) {
      expect(failureGuidanceKind({ code: "pipeline_failed", message: "IO error", failedStage, classificationVersion: 1 })).toBe("generic");
      expect(errorSummaryKey(mixed, { classificationVersion: 1 })).toBe("failure.genericTitle");
    }
  });
  it("leaves persisted legacy classifications and summaries unchanged", () => {
    const legacy = { classification: "brush_dataset", classification_is_heuristic: true };
    expect(taskFailureContext(legacy)).toBeUndefined();
    expect(errorSummaryKey(mixed, taskFailureContext(legacy))).toBe("error.summaryDataset");
    expect(failureCopy("mapper_storage").title).toBe("failure.mapperTitle");
    expect(failureCopy("brush_dataset").title).toBe("failure.brushDatasetTitle");
  });
  it("preserves precise native status and ignores malformed optional values", () => {
    expect(pipelineCommandError({ code: "pipeline_failed", message: "native exit", failureKind: "colmap_crash", classificationVersion: 1, classificationIsHeuristic: false,
      termination: { exitCode: -1073740940, windowsStatus: "0xC0000374" } })).toMatchObject({ failureKind: "colmap_crash", classificationVersion: 1, classificationIsHeuristic: false,
      termination: { exitCode: -1073740940, windowsStatus: "0xC0000374" } });
    expect(pipelineCommandError({ code: "pipeline_failed", message: "exit", classificationVersion: -1, termination: { windowsStatus: "arbitrary", signal: -1 } })).toMatchObject({ classificationVersion: undefined, termination: { windowsStatus: undefined, signal: undefined } });
  });
  it("does not show failure guidance for cancellation", () => {
    expect(failureGuidanceKind({ code: "cancelled", message: "cancelled", classificationVersion: 1 })).toBeNull();
  });
  it("keeps unclassified strings and IPC errors generic instead of guessing from the UI stage", () => {
    expect(failureGuidanceKind("COLMAP exited with an unknown status")).toBe("generic");
    expect(failureGuidanceKind(new Error("IO error while waiting for the backend"))).toBe("generic");
    expect(failureGuidanceKind({ code: "pipeline_failed", message: "read failed", failedStage: "reconstructing" })).toBe("generic");
  });
});
