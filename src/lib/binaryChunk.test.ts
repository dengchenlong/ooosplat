import { describe, expect, it } from "vitest";
import { ownedBinaryChunk } from "./binaryChunk";

describe("ownedBinaryChunk", () => {
  it("normalizes Tauri ArrayBuffer responses into transferable bytes", () => {
    const source = new Uint8Array([1, 2, 3]);
    const normalized = ownedBinaryChunk(source.buffer);
    expect([...normalized]).toEqual([1, 2, 3]);
    expect(normalized.buffer).not.toBe(source.buffer);
  });

  it("copies Uint8Array responses instead of transferring their backing storage", () => {
    const source = new Uint8Array([4, 5, 6]);
    const normalized = ownedBinaryChunk(source);
    source[0] = 9;
    expect([...normalized]).toEqual([4, 5, 6]);
  });
});
