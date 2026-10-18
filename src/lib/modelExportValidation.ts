import type { ChunkData } from "@playcanvas/splat-transform";

// The locked SPZ writer uses signed 24-bit positions with 12 fractional bits.
// Never silently wrap outlying points or change the user's coordinate system.
export function assertSpzPositions(values: Float32Array) {
  const limit = 0x7fffff / 4096;
  if (values.some(value => !Number.isFinite(value) || Math.abs(value) > limit)) throw new Error("SPZ_POSITION_RANGE");
}

export function validateDecodedLayer(data: Pick<ChunkData, "data" | "stride" | "count" | "layer" | "fields">) {
  const values = new Float32Array(data.data, 0, data.count * data.stride / 4);
  const opacityOffset = data.layer === "geometric" ? data.fields.opacity.byteOffset / 4 : -1;
  for (let i = 0; i < values.length; i++) {
    // A quantized alpha of 0/1 has a valid infinite logit. NaN is never valid.
    const opacity = opacityOffset >= 0 && i % (data.stride / 4) === opacityOffset;
    if (Number.isNaN(values[i]) || (!opacity && !Number.isFinite(values[i]))) throw new Error("Encoded model contains invalid Gaussian attributes");
  }
}
