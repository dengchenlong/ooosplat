import { describe, expect, it } from "vitest";
import { assertSpzPositions, validateDecodedLayer } from "./modelExportValidation";
describe("encoded Gaussian validation", () => {
  it("rejects SPZ integer overflow without clipping, scaling or removing points", () => {
    expect(() => assertSpzPositions(new Float32Array([0, 100, -2047, 2047]))).not.toThrow();
    for (const value of [2048, -2048, 5000, NaN, Infinity]) expect(() => assertSpzPositions(new Float32Array([value]))).toThrow("SPZ_POSITION_RANGE");
  });
  it("accepts saturated alpha logits but rejects invalid positions, scales, SH and NaN opacity", () => {
    const values = new Float32Array([1, 0, 0, 0, -2, -2, -2, Infinity]);
    const data = { data: values.buffer, stride: 32, count: 1, layer: "geometric" as const, fields: { opacity: { byteOffset: 28, components: 1, type: "float32" as const } } };
    expect(() => validateDecodedLayer(data)).not.toThrow();
    values[7] = -Infinity; expect(() => validateDecodedLayer(data)).not.toThrow();
    values[7] = NaN; expect(() => validateDecodedLayer(data)).toThrow();
    values[7] = 0; values[4] = Infinity; expect(() => validateDecodedLayer(data)).toThrow();
    for (const layer of ["position", "color"] as const) expect(() => validateDecodedLayer({ ...data, layer })).toThrow();
  });
});
