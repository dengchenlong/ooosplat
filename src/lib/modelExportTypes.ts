export type ModelFormat = "ply" | "compressedPly" | "spz" | "sog";
export interface PreparedModelExport {
    exportId: string;
    fileSize: number;
    splatCount: number;
    shBands: number;
    estimatedMemoryBytes: number;
    memoryBudgetBytes: number;
    compressionAllowed: boolean;
}
export interface ModelExportResult {
    path: string;
    fileSize: number;
    splatCount: number;
    format: ModelFormat;
}
export interface ModelValidation {
    splatCount: number;
    shBands: number;
}
export interface ModelProgress {
    exportId: string;
    phase: "preparing" | "encoding" | "validating" | "publishing";
    progress: number | null;
    detail?: string;
}
export interface ModelWorkerStart {
    type: "start";
    exportId: string;
    format: Exclude<ModelFormat, "ply">;
    filename: string;
    sourceSize: number;
    splatCount: number;
    shBands: number;
}
