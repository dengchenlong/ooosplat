// Test-only: compare decoded positions independently of codec ordering and emit
// canonical round-trip PLYs for visual checks. Never runs in the application.
// Usage: node scripts/inspect-model-export.mjs <reference.ply> <exports-directory>
import { readFile as readBytes, writeFile } from "node:fs/promises";
import path from "node:path";
import { readFile, readFileInfo, MemoryReadFileSystem, MemoryFileSystem, materializeToDataTable, createChunkDataPool, bakeTransform, Transform, writeSource, WorkerQueue, WebPCodec } from "@playcanvas/splat-transform";
WorkerQueue.maxWorkers = 0;
// Node verification also uses the already installed local codec, not the CDN.
WebPCodec.wasmUrl = path.resolve("node_modules/@playcanvas/splat-transform/lib/webp.wasm");
const [reference, directory] = process.argv.slice(2);
async function decode(filename, inputFormat, output) {
  const fs = new MemoryReadFileSystem(); fs.set(filename, await readBytes(filename));
  const options = { filename, inputFormat, fileSystem: fs };
  const info = await readFileInfo(options), [raw] = await readFile(options);
  const source = bakeTransform(raw, Transform.PLY), pool = createChunkDataPool({ maxPooledBytes: 0 });
  try {
    const table = await materializeToDataTable(source, pool, new Set(["position"]));
    if (output) {
      const out = new MemoryFileSystem(); await writeSource({ filename: "model.ply", outputFormat: "ply", source, pool, options: {} }, out);
      await writeFile(output, out.results.get("model.ply"));
    }
    return { info, xyz: ["x", "y", "z"].map(name => table.getColumnByName(name).data) };
  } finally { await raw.close(); pool.destroy(); }
}
const original = await decode(reference, "ply");
const { xyz } = original;
const span = Math.max(...xyz.map(a => { let min = Infinity, max = -Infinity; for (const x of a) { min = Math.min(min, x); max = Math.max(max, x); } return max - min; }));
function build(indices, depth = 0) {
  if (!indices.length) return null;
  const axis = depth % 3; indices.sort((a, b) => xyz[axis][a] - xyz[axis][b]);
  const middle = indices.length >> 1;
  return { index: indices[middle], axis, left: build(indices.slice(0, middle), depth + 1), right: build(indices.slice(middle + 1), depth + 1) };
}
const tree = build(Array.from({ length: original.info.numGaussians }, (_, i) => i));
function nearest(node, point, best = Infinity) {
  if (!node) return best;
  let squared = 0; for (let axis = 0; axis < 3; axis++) squared += (point[axis] - xyz[axis][node.index]) ** 2;
  best = Math.min(best, squared);
  const delta = point[node.axis] - xyz[node.axis][node.index];
  best = nearest(delta < 0 ? node.left : node.right, point, best);
  if (delta ** 2 < best) best = nearest(delta < 0 ? node.right : node.left, point, best);
  return best;
}
const results = [];
for (const [name, format] of [["edit.compressed.ply", "ply"], ["edit.spz", "spz"], ["edit.sog", "sog"]]) {
  const filename = path.join(directory, name);
  try { if (!(await readBytes(filename)).length) continue; } catch { continue; }
  const result = await decode(filename, format, path.join(directory, `${name}.roundtrip.ply`));
  if (result.info.numGaussians !== original.info.numGaussians || result.info.shBands !== original.info.shBands) throw new Error(`${name}: count/SH changed`);
  let maximum = 0, total = 0;
  for (let i = 0; i < result.info.numGaussians; i++) { const squared = nearest(tree, result.xyz.map(a => a[i])); maximum = Math.max(maximum, Math.sqrt(squared)); total += squared; }
  const tolerance = format === "spz" ? Math.sqrt(3) / 4096 + 1e-5 : span * 0.002 + 1e-5;
  if (maximum > tolerance) throw new Error(`${name}: position error ${maximum} exceeds ${tolerance}`);
  const r = { name, count: result.info.numGaussians, shBands: result.info.shBands, maxPositionError: maximum, rmsPositionError: Math.sqrt(total / result.info.numGaussians), tolerance };
  results.push(r); console.log(JSON.stringify(r));
}
await writeFile(path.join(directory, "roundtrip.json"), JSON.stringify(results, null, 2));
