export type ModelFormat = "ply" | "compressedPly" | "spz" | "sog";
export interface SpzAdjustment {
    center: [number, number, number];
    scale: number;
}
export interface PreparedModelExport {
    exportId: string;
    fileSize: number;
    splatCount: number;
    shBands: number;
}
export interface ModelExportResult {
    path: string;
    fileSize: number;
    splatCount: number;
    format: ModelFormat;
    coordinateAdjustment?: SpzAdjustment;
}
export interface ModelValidation {
    splatCount: number;
    shBands: number;
}
interface ModelProgressBase {
    exportId: string;
    progress: number | null;
    detail?: string;
}
export type ModelProgress = ModelProgressBase & (
    | { phase: "preparing" | "encoding" | "validating" | "publishing" }
    | { phase: "awaitingConfirmation"; confirmationId: number; adjustment: SpzAdjustment }
);
export interface ModelWorkerStart {
    type: "start";
    exportId: string;
    format: Exclude<ModelFormat, "ply">;
    sourceSize: number;
    splatCount: number;
    shBands: number;
}
