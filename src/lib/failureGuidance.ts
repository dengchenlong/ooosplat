import type { TranslationKey } from "../i18n";
import { pipelineCommandError, type FailureContext, type PipelineFailureKind } from "./pipelineError";

export type FailureGuidanceKind = PipelineFailureKind | "generic";

export function failureGuidanceKind(error: unknown): FailureGuidanceKind | null {
  const structured = pipelineCommandError(error);
  if (structured?.code === "cancelled") return null;
  // Old explicit classifications remain usable. Neither the display string nor
  // the last UI stage is evidence of the cause of an unclassified failure.
  return structured?.failureKind ?? "generic";
}

export function failureCopy(kind: FailureGuidanceKind, version?: number): { title: TranslationKey; description: TranslationKey; tips: TranslationKey[] } {
  if (version != null) {
    const keys: Partial<Record<FailureGuidanceKind, [TranslationKey, TranslationKey]>> = {
      brush_out_of_memory: ["failure.oomTitle", "failure.oomDescription"],
      brush_resource_limit: ["failure.resourceLimitTitle", "failure.resourceLimitDescription"],
      colmap_crash: ["failure.colmapCrashTitle", "failure.colmapCrashDescription"],
      mapper_storage: ["failure.storageTitleV1", "failure.storageDescriptionV1"],
      brush_dataset: ["failure.datasetTitleV1", "failure.datasetDescriptionV1"],
    };
    const copy = keys[kind];
    if (copy) return { title: copy[0], description: copy[1], tips: [] };
  }
  switch (kind) {
    case "mapper_source": return { title: "failure.mapperTitle", description: "failure.mapperSource", tips: ["failure.mapperTip1", "failure.mapperTip2", "failure.mapperTip3"] };
    case "mapper_storage": return { title: "failure.mapperTitle", description: "failure.mapperStorage", tips: ["failure.storageTip1", "failure.storageTip2"] };
    case "brush_dataset": return { title: "failure.brushDatasetTitle", description: "failure.brushDataset", tips: ["failure.datasetTip1", "failure.datasetTip2"] };
    case "brush_device_lost": return { title: "failure.brushDeviceLostTitle", description: "failure.brushDeviceLost", tips: ["failure.deviceLostTip1", "failure.deviceLostTip2", "failure.deviceLostTip3"] };
    case "brush_gpu": return { title: "failure.brushTitle", description: "failure.brushGpu", tips: ["failure.brushTip1", "failure.brushTip2", "failure.brushTip3"] };
    default: return { title: "failure.genericTitle", description: "failure.genericDescription", tips: ["failure.genericTip1"] };
  }
}

export function classifiedSummary(context?: FailureContext): TranslationKey | null {
  if (context?.classificationVersion == null) return null;
  return failureCopy(context.failureKind ?? "generic", context.classificationVersion).title;
}
