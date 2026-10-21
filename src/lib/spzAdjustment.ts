import type { ChunkData, ChunkSource } from "@playcanvas/splat-transform";
import type { SpzAdjustment } from "./modelExportTypes";
import { assertSpzPositions, validateDecodedLayer } from "./modelExportValidation";

// The locked SPZ encoder uses signed 24-bit coordinates with 12 fractional bits.
export const SPZ_POSITION_LIMIT = 0x7fffff / 4096;
export const SPZ_SAFE_POSITION_LIMIT = (0x7fffff - 2) / 4096;
export const SPZ_MIN_LOG_SCALE = -10;
export const SPZ_MAX_LOG_SCALE = 255 / 16 - 10;
export interface SpzBounds {
    min: [number, number, number];
    max: [number, number, number];
    minLogScale: number;
    maxLogScale: number;
    count: number;
}
export const createSpzBounds = (): SpzBounds => ({
    min: [Infinity, Infinity, Infinity], max: [-Infinity, -Infinity, -Infinity],
    minLogScale: Infinity, maxLogScale: -Infinity, count: 0,
});

export function includeSpzChunk(bounds: SpzBounds, position: ChunkData, geometric: ChunkData) {
    validateDecodedLayer(position); validateDecodedLayer(geometric);
    if (position.count !== geometric.count) throw new Error("Invalid SPZ chunk counts");
    const p = new Float32Array(position.data), g = new Float32Array(geometric.data);
    const ps = position.stride / 4, gs = geometric.stride / 4;
    const po = position.fields.position.byteOffset / 4, so = geometric.fields.scale.byteOffset / 4;
    for (let i = 0; i < position.count; i++) {
        for (let axis = 0; axis < 3; axis++) {
            const value = p[i * ps + po + axis], logScale = g[i * gs + so + axis];
            bounds.min[axis] = Math.min(bounds.min[axis], value);
            bounds.max[axis] = Math.max(bounds.max[axis], value);
            bounds.minLogScale = Math.min(bounds.minLogScale, logScale);
            bounds.maxLogScale = Math.max(bounds.maxLogScale, logScale);
        }
    }
    bounds.count += position.count;
}

export function planSpzAdjustment(bounds: SpzBounds): SpzAdjustment | null {
    if (!bounds.count || ![...bounds.min, ...bounds.max, bounds.minLogScale, bounds.maxLogScale].every(Number.isFinite)) {
        throw new Error("Invalid SPZ model bounds");
    }
    if (bounds.min.every(v => v >= -SPZ_POSITION_LIMIT) && bounds.max.every(v => v <= SPZ_POSITION_LIMIT)) return null;
    const center = bounds.min.map((v, i) => v / 2 + bounds.max[i] / 2) as SpzAdjustment["center"];
    const radius = Math.max(...bounds.max.map((v, i) => Math.max(v - center[i], center[i] - bounds.min[i])));
    return { center, scale: radius === 0 ? 1 : Math.min(1, SPZ_SAFE_POSITION_LIMIT / radius) };
}

export function assertSpzAdjustmentSizes(bounds: SpzBounds, adjustment: SpzAdjustment) {
    if (!Number.isFinite(adjustment.scale) || adjustment.scale <= 0 || adjustment.scale > 1) throw new Error("Invalid SPZ scale factor");
    if (adjustment.scale === 1) return; // Translation preserves the existing codec's scale quantization.
    const delta = Math.log(adjustment.scale);
    if (Math.fround(bounds.minLogScale + delta) < SPZ_MIN_LOG_SCALE || Math.fround(bounds.maxLogScale + delta) > SPZ_MAX_LOG_SCALE) {
        throw new Error("SPZ_SCALE_RANGE");
    }
}

// Source must already be in PLY space. Parent reads refill buffers before every
// application, including gathers and repeated reads, so transforms never accumulate.
export function adjustedSpzSource(source: ChunkSource, adjustment: SpzAdjustment): ChunkSource {
    const delta = Math.log(adjustment.scale);
    return {
        meta: source.meta,
        close: () => source.close(),
        async read(request) {
            await source.read(request);
            if (request.position) {
                const cd = request.position, values = new Float32Array(cd.data), offset = cd.fields.position.byteOffset / 4;
                for (let i = 0; i < cd.count; i++) for (let axis = 0; axis < 3; axis++) {
                    const index = i * cd.stride / 4 + offset + axis;
                    values[index] = (values[index] - adjustment.center[axis]) * adjustment.scale;
                }
                assertSpzPositions(new Float32Array(cd.data, 0, cd.count * cd.stride / 4));
            }
            if (request.geometric && adjustment.scale !== 1) {
                const cd = request.geometric, values = new Float32Array(cd.data), offset = cd.fields.scale.byteOffset / 4;
                for (let i = 0; i < cd.count; i++) for (let axis = 0; axis < 3; axis++) {
                    const index = i * cd.stride / 4 + offset + axis;
                    values[index] += delta;
                    if (values[index] < SPZ_MIN_LOG_SCALE || values[index] > SPZ_MAX_LOG_SCALE) throw new Error("SPZ_SCALE_RANGE");
                }
            }
        },
    };
}
