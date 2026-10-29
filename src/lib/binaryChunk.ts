export type BinaryChunk = ArrayBuffer | Uint8Array;

/** Normalize Tauri raw IPC responses and test doubles to an owned byte view. */
export function ownedBinaryChunk(chunk: BinaryChunk): Uint8Array<ArrayBuffer> {
  return new Uint8Array(chunk instanceof Uint8Array ? chunk : new Uint8Array(chunk));
}
