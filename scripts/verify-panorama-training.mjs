// Real, offline Brush recipe diagnostic. Reads an existing dataset; writes only
// a new .cache directory. Never edits project state, models or initialization.
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { brushLock, verifyBrushRuntime } from "./brush-runtime.mjs";

const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const argv = process.argv.slice(2);
const option = (name, fallback) => {
  const i = argv.indexOf(name);
  return i < 0 ? fallback : argv[i + 1];
};
const dataset = path.resolve(option("--dataset", ""));
const selection = option("--quality", "all");
const thresholdControl = argv.includes("--threshold-control");
if (thresholdControl && selection !== "high") throw new Error("Threshold control is only valid for --quality high");
if (!argv.includes("--dataset") || !["all", "fast", "balanced", "high"].includes(selection)) {
  throw new Error("Usage: node scripts/verify-panorama-training.mjs --dataset <existing panorama-training> [--quality all|fast|balanced|high]");
}
const manifest = JSON.parse(fs.readFileSync(path.join(dataset, "manifest.json"), "utf8"));
const config = JSON.parse(fs.readFileSync(path.join(workspace, "config/pipeline-optimization.json"), "utf8")).panoramaTraining;
const binary = path.join(workspace, "engines/brush/brush_app.exe");
verifyBrushRuntime(path.dirname(binary), brushLock("windows"));
const root = path.join(workspace, ".cache/panorama-training-v1", `${Date.now()}-${selection}${thresholdControl ? "-threshold-control" : ""}`);
fs.mkdirSync(root, { recursive: true });
console.log(`Diagnostic output: ${root}`);
const results = [];
const protectedFiles = ["manifest.json", "signature.json", "sparse/0/cameras.bin", "sparse/0/images.bin", "sparse/0/points3D.bin"];
const fingerprint = file => crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
const sourceBefore = Object.fromEntries(protectedFiles.map(name => [name, fingerprint(path.join(dataset, name))]));
const initialization = fs.openSync(path.join(dataset, "sparse/0/points3D.bin"), "r");
const initialCountBuffer = Buffer.alloc(8);
fs.readSync(initialization, initialCountBuffer, 0, 8, 0);
fs.closeSync(initialization);
const initialPoints = Number(initialCountBuffer.readBigUInt64LE());
if (initialPoints > 10_000_000) throw new Error("Initial points exceed hard budget");

function validatePly(file) {
  const fd = fs.openSync(file, "r");
  try {
    const header = Buffer.alloc(16384);
    const size = fs.readSync(fd, header, 0, header.length, 0);
    const end = header.subarray(0, size).indexOf("end_header\n") + "end_header\n".length;
    if (end < "end_header\n".length) throw new Error("Missing PLY header");
    const text = header.subarray(0, end).toString("ascii");
    if (!text.includes("format binary_little_endian 1.0")) throw new Error("Unsupported PLY encoding");
    const count = Number(text.match(/element vertex (\d+)/)?.[1]);
    const properties = [...text.matchAll(/^property (\w+) (\w+)/gm)];
    if (!count || properties.some(p => p[1] !== "float")) throw new Error("Invalid PLY vertex schema");
    const stride = properties.length * 4;
    const bytes = fs.fstatSync(fd).size;
    if (bytes !== end + count * stride) throw new Error("Incomplete PLY payload");
    const block = Buffer.alloc(1024 * 1024);
    for (let offset = end; offset < bytes;) {
      const length = fs.readSync(fd, block, 0, Math.min(block.length, bytes - offset), offset);
      if (!length || length % 4) throw new Error("Invalid PLY block");
      for (let i = 0; i < length; i += 4) {
        if (!Number.isFinite(block.readFloatLE(i))) throw new Error("Non-finite Gaussian attribute");
      }
      offset += length;
    }
    return { count, bytes, shDegree: Math.sqrt(properties.filter(p => p[2].startsWith("f_rest_")).length / 3 + 1) - 1 };
  } finally { fs.closeSync(fd); }
}

for (const quality of selection === "all" ? ["fast", "balanced", "high"] : [selection]) {
  const recipe = structuredClone(config[quality]);
  if (thresholdControl) recipe.densification.growthGradThreshold = 0.0008;
  const output = path.join(root, quality);
  fs.mkdirSync(output);
  const freeBytes = fs.statfsSync(output).bavail * fs.statfsSync(output).bsize;
  if (freeBytes < 8 * 1024 ** 3) throw new Error("Diagnostic needs at least 8 GiB free disk");
  const manualCap = recipe.maxSplatsWithoutOptimization ?? { fast: 1_000_000, balanced: 2_000_000, high: 3_000_000 }[quality];
  const cap = Math.max(manualCap, initialPoints);
  if (!Number.isSafeInteger(cap) || cap <= 0 || cap > 10_000_000) throw new Error("Invalid diagnostic point budget");
  const faceLimit = { fast: 512, balanced: 768, high: 1024 }[quality];
  const args = ["--total-train-iters", recipe.totalSteps, "--max-resolution", Math.min(faceLimit, manifest.config.faceSize),
    "--max-splats", cap, "--refine-every", recipe.refineEvery,
    "--growth-grad-threshold", recipe.densification.growthGradThreshold,
    "--growth-select-fraction", recipe.densification.growthSelectFraction,
    "--growth-start-iter", 0, "--growth-stop-iter", recipe.densification.growthStopIter,
    "--sh-degree", 3, "--max-scene-batch-cache-size", "2GiB",
    "--export-every", recipe.totalSteps, "--export-path", output, "--export-name", "final.ply", dataset].map(String);
  const result = { quality, recipeVersion: config.recipeVersion, recipe, args, dataset, initialPoints,
    experimentalThresholdControl: thresholdControl,
    cachedFaceSize: manifest.config.faceSize, automaticOptimization: false, growth: [], resourceSamples: [],
    resourceScope: "GPU samples are whole-device usage (not isolated Brush allocations); process memory is Brush working set/private bytes." };
  fs.writeFileSync(path.join(output, "launch.json"), JSON.stringify(result, null, 2));
  console.log(`Starting ${quality}: ${recipe.totalSteps} steps, cap ${cap}, face ${Math.min(faceLimit, manifest.config.faceSize)}`);
  const log = fs.createWriteStream(path.join(output, "brush.log"));
  const started = Date.now();
  const child = spawn(binary, args, { cwd: output, windowsHide: true,
    env: { ...process.env, RUST_LOG: "brush_cli=info,brush_process=info,cubecl_wgpu=info,burn_wgpu=info", RUST_BACKTRACE: "1" } });
  let pending = "";
  const consume = chunk => {
    log.write(chunk);
    pending += chunk.toString();
    const lines = pending.split(/\r?\n/);
    pending = lines.pop();
    for (const line of lines) {
      const refine = line.match(/Refine iter (\d+), (\d+) splats/);
      if (refine) {
        const point = { iteration: Number(refine[1]), splats: Number(refine[2]), elapsedMs: Date.now() - started };
        result.growth.push(point);
        if (point.iteration % 2000 === 1) console.log(`${quality}: step ${point.iteration}, ${point.splats} splats`);
      }
    }
  };
  child.stdout.on("data", consume);
  child.stderr.on("data", consume);
  let sampling = false;
  const timer = setInterval(async () => {
    if (sampling || !child.pid) return;
    sampling = true;
    try {
      const sampled = await new Promise(resolve => {
        const p = spawn("nvidia-smi", ["--query-gpu=index,name,memory.used,memory.total", "--format=csv,noheader,nounits"], { windowsHide: true });
        let out = "";
        p.stdout.on("data", b => { out += b; });
        p.on("error", () => resolve(null));
        p.on("close", code => resolve(code === 0 ? out.trim() : null));
      });
      const memory = spawnSync("powershell", ["-NoProfile", "-Command", `$p=Get-Process -Id ${child.pid} -ErrorAction SilentlyContinue; if ($p) { "$($p.WorkingSet64),$($p.PrivateMemorySize64)" }`],
        { windowsHide: true, encoding: "utf8", timeout: 5000 });
      const values = memory.stdout?.trim().split(",").map(Number);
      result.resourceSamples.push({ elapsedMs: Date.now() - started, gpu: sampled,
        workingSetBytes: values?.[0] || null, privateBytes: values?.[1] || null });
    } finally { sampling = false; }
  }, 5000);
  const stop = () => { if (child.pid) spawnSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { windowsHide: true }); };
  const interrupt = () => { result.cancelled = true; stop(); };
  process.once("SIGINT", interrupt);
  const timeout = setTimeout(() => { result.timedOut = true; stop(); }, 90 * 60_000);
  try {
    result.exitCode = await new Promise((resolve, reject) => { child.on("error", reject); child.on("close", resolve); });
    result.durationMs = Date.now() - started;
    if (result.exitCode === 0) {
      result.output = validatePly(path.join(output, "final.ply"));
      if (result.output.shDegree !== 3 || result.output.count > cap) throw new Error("SH/cap mismatch");
      result.reachedFiveMillion = result.output.count >= 5_000_000;
    }
    console.log(`${quality} finished: ${JSON.stringify({ exitCode: result.exitCode, output: result.output, durationMs: result.durationMs })}`);
  } catch (error) { result.error = String(error); }
  finally {
    clearInterval(timer); clearTimeout(timeout); process.removeListener("SIGINT", interrupt); log.end();
    result.peakWorkingSetBytes = Math.max(0, ...result.resourceSamples.map(s => s.workingSetBytes ?? 0));
    result.peakPrivateBytes = Math.max(0, ...result.resourceSamples.map(s => s.privateBytes ?? 0));
    result.peakWholeGpuUsedMiB = Math.max(0, ...result.resourceSamples.flatMap(s => (s.gpu ?? "").split("\n").map(line => Number(line.split(",").at(-2)) || 0)));
    fs.writeFileSync(path.join(output, "result.json"), JSON.stringify(result, null, 2));
    results.push(result);
  }
  if (result.cancelled) break;
}
const sourceAfter = Object.fromEntries(protectedFiles.map(name => [name, fingerprint(path.join(dataset, name))]));
const sourceUnchanged = JSON.stringify(sourceBefore) === JSON.stringify(sourceAfter);
fs.writeFileSync(path.join(root, "summary.json"), JSON.stringify({ sourceBefore, sourceAfter, sourceUnchanged, results }, null, 2));
if (!sourceUnchanged) throw new Error("Source dataset changed during diagnostic");
if (results.some(r => r.error || r.exitCode !== 0)) process.exitCode = 1;
