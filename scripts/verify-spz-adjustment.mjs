// Real worker + independent readback/render verification. Writes only isolated
// .cache fixtures; never mutates input or registers projects. No test consent is
// assumed by the application: this script explicitly supplies test-only answers.
// Usage: node scripts/verify-spz-adjustment.mjs <small-standard-real-model.ply>
import { readFile as readBytes, writeFile, mkdir, stat } from "node:fs/promises";
import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import path from "node:path";
import { readFile, readFileInfo, MemoryReadFileSystem, MemoryFileSystem, createChunkDataPool, bakeTransform, Transform, materializeToDataTable, writeSource } from "@playcanvas/splat-transform";

const root = process.cwd(), input = path.resolve(process.argv[2]);
const out = path.join(root, ".cache", `spz-adjustment-${Date.now()}`); await mkdir(out, { recursive: true });
const original = await readBytes(input), originalHash = createHash("sha256").update(original).digest("hex");
const headerEnd = original.indexOf("end_header\n") + 11;
const header = original.subarray(0, headerEnd).toString();
const fields = [...header.matchAll(/property float (\S+)/g)].map(m => m[1]);
const count = Number(/element vertex (\d+)/.exec(header)?.[1]), stride = fields.length * 4;
if (!count || count > 100_000 || !header.includes("binary_little_endian") || original.length !== headerEnd + count * stride) throw Error("Expected a small, float32 standard Gaussian PLY");
const offsets = ["x", "y", "z", "scale_0", "scale_1", "scale_2"].map(key => fields.indexOf(key) * 4);
if (offsets.some(n => n < 0)) throw Error("Missing Gaussian fields");
function fixture(factor, translation, scaleOverride) {
    const b = Buffer.from(original);
    for (let i = 0; i < count; i++) for (let axis = 0; axis < 3; axis++) {
        const p = headerEnd + i * stride + offsets[axis], s = headerEnd + i * stride + offsets[axis + 3];
        b.writeFloatLE(b.readFloatLE(p) * factor + translation[axis], p);
        b.writeFloatLE(scaleOverride ?? b.readFloatLE(s) + Math.log(factor), s);
    }
    return b;
}
async function command(script, args, extra = {}) {
    const env = { ...process.env, ...extra }; delete env.SMOKE_POINTS; delete env.MODEL_VIEW_ENDPOINT; delete env.CAMERA_POSES_JSON;
    return new Promise((resolve, reject) => {
        const p = spawn(process.execPath, [script, ...args], { cwd: root, env, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
        let stdout = "", stderr = "";
        p.stdout.on("data", b => { stdout += b; }); p.stderr.on("data", b => { stderr = (stderr + b).slice(-8192); });
        p.on("error", reject); p.on("exit", code => resolve({ code, stdout, stderr }));
    });
}
async function decode(filename, inputFormat) {
    const fs = new MemoryReadFileSystem(); fs.set(filename, await readBytes(filename));
    const options = { filename, inputFormat, fileSystem: fs }, info = await readFileInfo(options), [raw] = await readFile(options);
    const pool = createChunkDataPool({ maxPooledBytes: 0 }), source = bakeTransform(raw, Transform.PLY);
    try {
        const table = await materializeToDataTable(source, pool);
        const columns = Object.fromEntries(["x", "y", "z", "scale_0", "scale_1", "scale_2"].map(k => [k, table.getColumnByName(k).data]));
        const fsOut = new MemoryFileSystem(); await writeSource({ filename: "roundtrip.ply", outputFormat: "ply", source, pool, options: {} }, fsOut);
        await writeFile(filename + ".roundtrip.ply", fsOut.results.get("roundtrip.ply"));
        return { info, columns, roundtrip: filename + ".roundtrip.ply" };
    } finally { await raw.close(); pool.destroy(); }
}

const results = [];
for (const test of [
    { name: "unchanged", factor: 1, translation: [0, 0, 0], action: "accept" },
    { name: "translated", factor: 1, translation: [10000, -10000, 10000], action: "accept" },
    { name: "scaled", factor: 64, translation: [10000, -10000, 10000], action: "accept" },
    { name: "cancelled", factor: 1, translation: [10000, -10000, 10000], action: "cancel" },
    { name: "unsafe-scale", factor: 64, translation: [10000, -10000, 10000], action: "accept", scaleOverride: -9.9 },
]) {
    const source = path.join(out, `${test.name}.ply`), sourceBytes = fixture(test.factor, test.translation, test.scaleOverride);
    await writeFile(source, sourceBytes);
    const run = await command("scripts/smoke-model-export.mjs", [source], { SMOKE_FORMATS: "spz", SMOKE_SPZ_ACTION: test.action });
    const directory = run.stdout.split(/\r?\n/).findLast(line => line.startsWith(path.join(root, ".cache", "model-export-")));
    if (!directory) throw Error(`No evidence directory: ${run.stdout} ${run.stderr}`);
    const report = JSON.parse(await readBytes(path.join(directory, "results.json")));
    if (!Buffer.from(await readBytes(source)).equals(sourceBytes)) throw Error("Test input changed");
    if (test.name === "cancelled" || test.name === "unsafe-scale") {
        const expected = test.name === "cancelled" ? "AbortError" : "SPZ_SCALE_RANGE";
        if (run.code === 0 || !report.error?.includes(expected) || report.outputs.spz.started || report.results.length || report.confirmations.length !== 1) throw Error(`Expected safe refusal: ${JSON.stringify(report)}`);
        if (await stat(report.outputs.spz.path).then(() => true, () => false)) throw Error("Unexpected output before encoding");
        const result = { name: test.name, directory, error: report.error, outputCreated: false }; results.push(result); console.log(JSON.stringify(result)); continue;
    }
    if (run.code !== 0 || report.error || report.results.length !== 1) throw Error(`Worker failed: ${run.stdout} ${run.stderr}`);
    const adjusted = report.results[0].coordinateAdjustment;
    if (report.confirmations.length !== (test.name === "unchanged" ? 0 : 1) || (test.name === "translated" && adjusted?.scale !== 1) || (test.name === "scaled" && !(adjusted?.scale < 1))) throw Error("Unexpected normalization/consent");
    const decoded = await decode(report.outputs.spz.path, "spz");
    if (decoded.info.numGaussians !== count || decoded.info.shBands !== report.fixture.sh) throw Error("Count/SH changed");
    const expected = [], logDelta = Math.log(adjusted?.scale ?? 1);
    for (let i=0;i<count;i++) expected.push(offsets.map((offset,k) => {
        const v=sourceBytes.readFloatLE(headerEnd+i*stride+offset);
        return k<3 ? Math.fround((v-(adjusted?.center[k] ?? 0))*(adjusted?.scale ?? 1)) : Math.fround(v+logDelta);
    }));
    // The pinned SPZ encoder preserves input order. Compare every point/scale,
    // not just the AABB; a reorder would fail rather than hide changed data.
    let positionError=0, scaleError=0;
    const names=["x","y","z","scale_0","scale_1","scale_2"];
    for(let i=0;i<count;i++) for(let k=0;k<6;k++) {
        const error=Math.abs(decoded.columns[names[k]][i]-expected[i][k]);
        if(k<3) positionError=Math.max(positionError,error); else scaleError=Math.max(scaleError,error);
    }
    if(positionError > 1/4096+0.0002 || scaleError > 1/32+0.0001) throw Error(`Quantization error: ${positionError}/${scaleError}`);
    const adjustmentFile=path.join(out,`${test.name}.render-adjustments.json`);
    await writeFile(adjustmentFile,JSON.stringify([null,adjusted]));
    const render=await command("scripts/render-model-export.mjs",[source,decoded.roundtrip],{MODEL_ADJUSTMENTS_JSON:adjustmentFile});
    if(render.code!==0) throw Error(`Render failed: ${render.stdout} ${render.stderr}`);
    const result={name:test.name,directory,count,sh:decoded.info.shBands,adjustment:adjusted,positionError,scaleError,render:render.stdout.trim()};
    results.push(result);console.log(JSON.stringify(result));
}
if(createHash("sha256").update(await readBytes(input)).digest("hex")!==originalHash) throw Error("Original source changed");
await writeFile(path.join(out,"verification.json"),JSON.stringify({input,originalHash,results},null,2));
console.log(out);
