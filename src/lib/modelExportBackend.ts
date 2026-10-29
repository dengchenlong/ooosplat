import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { GaussianTransform } from "../types/pipeline";
import type { ModelFormat, ModelValidation, ModelExportResult, PreparedModelExport } from "./modelExportTypes";
export const beginModelExport = (projectId: string, transform: GaussianTransform, editRevision: number, format: ModelFormat) => invoke<string>("begin_gaussian_model_export", { projectId, transform, editRevision, format });
export const prepareModelExport = (exportId: string) => invoke<PreparedModelExport>("prepare_gaussian_model_export", { exportId });
export const startModelEncoding = (exportId: string, format: ModelFormat) => invoke<string>("start_gaussian_model_encoding", { exportId, format });
export const completeModelExport = (exportId: string, validation: ModelValidation | null) => invoke<ModelExportResult>("complete_gaussian_model_export", { exportId, validation });
export const cancelModelExport = (exportId: string) => invoke<void>("cancel_gaussian_model_export", { exportId });
export const readModelExportChunk = (exportId: string, file: "source" | "output", offset: number, length: number) => invoke<ArrayBuffer>("read_gaussian_model_export_chunk", {}, { headers: { "x-ooosplat-export-id": exportId, "x-ooosplat-export-file": file, "x-ooosplat-offset": String(offset), "x-ooosplat-length": String(length) } });
export const writeModelExportChunk = (exportId: string, offset: number, bytes: Uint8Array) => invoke<number>("write_gaussian_model_export_chunk", bytes, { headers: { "x-ooosplat-export-id": exportId, "x-ooosplat-offset": String(offset) } });
export const onModelExportProgress = (callback: (event: {
    exportId: string;
    projectId: string;
    phase: "preparing" | "encoding";
    progress: number;
}) => void) => listen<{
    exportId: string;
    projectId: string;
    phase: "preparing" | "encoding";
    progress: number;
}>("gaussian-model-export-progress", event => callback(event.payload));
