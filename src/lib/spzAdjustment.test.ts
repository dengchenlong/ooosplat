import { describe, expect, it } from "vitest";
import type { ChunkData, ChunkSource } from "@playcanvas/splat-transform";
import { adjustedSpzSource, assertSpzAdjustmentSizes, createSpzBounds, includeSpzChunk, planSpzAdjustment, SPZ_POSITION_LIMIT, SPZ_SAFE_POSITION_LIMIT } from "./spzAdjustment";

const chunk = (values: number[], layer: "position" | "geometric" | "color"): ChunkData => {
    const width = layer === "position" ? 3 : layer === "geometric" ? 8 : 3;
    return { data: new Float32Array(values).buffer, stride: width * 4, count: values.length / width, layer,
        fields: layer === "position" ? { position: { byteOffset: 0, components: 3, type: "float32" } }
            : layer === "geometric" ? { scale: { byteOffset: 16, components: 3, type: "float32" }, opacity: { byteOffset: 28, components: 1, type: "float32" } } : {},
        release() {},
    } as ChunkData;
};
const geometry = (count: number, scale = -2) => chunk(Array.from({ length: count }, () => [1, 0, 0, 0, scale, scale, scale, 0]).flat(), "geometric");
const boundsFor = (points: number[], scale = -2) => {
    const bounds = createSpzBounds(); includeSpzChunk(bounds, chunk(points, "position"), geometry(points.length / 3, scale)); return bounds;
};

describe("SPZ export-only normalization", () => {
    it("keeps legal coordinates unchanged, including the encoder boundary", () => {
        expect(planSpzAdjustment(boundsFor([-SPZ_POSITION_LIMIT, 0, 0, SPZ_POSITION_LIMIT, 1, -1]))).toBeNull();
    });
    it("only translates distant compact scenes and single points", () => {
        expect(planSpzAdjustment(boundsFor([10000, 3, 4, 10010, 5, 6]))).toEqual({ center: [10005, 4, 5], scale: 1 });
        expect(planSpzAdjustment(boundsFor([1e6, -1e6, 5]))).toEqual({ center: [1e6, -1e6, 5], scale: 1 });
    });
    it("uniformly fits the complete bounds, including outliers and all chunks", () => {
        const bounds = boundsFor([-5000, -2, 0, 5000, 2, 0]);
        includeSpzChunk(bounds, chunk([1e7, 0, 0], "position"), geometry(1));
        const result = planSpzAdjustment(bounds)!;
        expect(bounds.count).toBe(3);
        expect(result.center).toEqual([4_997_500, 0, 0]);
        expect(result.scale).toBeCloseTo(SPZ_SAFE_POSITION_LIMIT / 5_002_500, 15);
        for (const x of [-5000, 5000, 1e7]) expect(Math.abs(Math.fround((x - result.center[0]) * result.scale))).toBeLessThanOrEqual(SPZ_POSITION_LIMIT);
    });
    it("rejects nonfinite data and empty models rather than normalizing them", () => {
        expect(() => planSpzAdjustment(createSpzBounds())).toThrow();
        for (const value of [NaN, Infinity, -Infinity]) {
            expect(() => boundsFor([value, 0, 0])).toThrow();
            expect(() => boundsFor([0, 0, 0], value)).toThrow();
        }
    });
    it("checks scale quantization before shrinking but does not change translation-only quantization", () => {
        const adjustment = { center: [0, 0, 0] as [number, number, number], scale: 0.5 };
        expect(() => assertSpzAdjustmentSizes(boundsFor([-5000, 0, 0], -2), adjustment)).not.toThrow();
        for (const logScale of [-9.9, 20]) expect(() => assertSpzAdjustmentSizes(boundsFor([-5000, 0, 0], logScale), adjustment)).toThrow("SPZ_SCALE_RANGE");
        expect(() => assertSpzAdjustmentSizes(boundsFor([10000, 0, 0], -20), { ...adjustment, scale: 1 })).not.toThrow();
        for (const scale of [0, -1, Infinity, NaN, 2]) expect(() => assertSpzAdjustmentSizes(boundsFor([0, 0, 0]), { ...adjustment, scale })).toThrow();
    });
    it("applies positions and log-scales once per read, preserving rotations, opacity, color and SH metadata", async () => {
        const originalP = new Float32Array([5000, 2, 3, 5010, 4, 6]), originalG = new Float32Array(geometry(2).data);
        const originalC = new Float32Array([.1, .2, .3, .4, .5, .6]);
        const parent = { meta: { shBands: 3, numGaussians: 2 }, close() {}, async read(request) {
            const indices = "indices" in request ? Array.from(request.indices.subarray(request.indexOffset, request.indexOffset + request.count)) : [0, 1];
            for (const [data, original, width] of [[request.position, originalP, 3], [request.geometric, originalG, 8], [request.color, originalC, 3]] as const) {
                if (data) indices.forEach((index, i) => new Float32Array(data.data).set(original.subarray(index * width, (index + 1) * width), i * width));
            }
        } } as ChunkSource;
        const view = adjustedSpzSource(parent, { center: [5000, 0, 0], scale: .5 });
        const position = chunk([0, 0, 0, 0, 0, 0], "position"), geometric = geometry(2), color = chunk([0, 0, 0, 0, 0, 0], "color");
        for (let repeat = 0; repeat < 2; repeat++) {
            await view.read({ chunkIndex: 0, position, geometric, color });
            expect(Array.from(new Float32Array(position.data))).toEqual([0, 1, 1.5, 5, 2, 3]);
            const g = new Float32Array(geometric.data);
            expect(g[4]).toBeCloseTo(-2 + Math.log(.5)); expect(g[0]).toBe(1); expect(g[7]).toBe(0);
            expect(new Float32Array(color.data)).toEqual(originalC);
        }
        await view.read({ indices: new Uint32Array([1, 0]), indexOffset: 0, count: 2, position });
        expect(Array.from(new Float32Array(position.data))).toEqual([5, 2, 3, 0, 1, 1.5]);
        expect(view.meta).toBe(parent.meta); expect(view.meta.shBands).toBe(3);
        expect(Array.from(originalP)).toEqual([5000, 2, 3, 5010, 4, 6]);
    });
});
