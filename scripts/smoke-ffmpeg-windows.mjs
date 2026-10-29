import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { assertWindowsFfmpegRelease } from "./ffmpeg-release-policy.mjs";

const manifest = JSON.parse(fs.readFileSync(new URL("../engines/manifest.json", import.meta.url), "utf8"));
const lock = manifest.engines.find(engine => engine.name === "FFmpeg / FFprobe");
assertWindowsFfmpegRelease(lock);
const runtime = path.resolve(process.argv[2] ?? "engines/ffmpeg");
for (const file of manifest.requiredFiles.filter(file => file.path.startsWith("engines/ffmpeg/"))) {
  const bytes = fs.readFileSync(path.join(runtime, path.basename(file.path)));
  assert.equal(createHash("sha256").update(bytes).digest("hex").toUpperCase(), file.sha256, file.path);
}
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "OOOSplat FFmpeg 中文 "));
function run(binary, args, binaryOutput = false) {
  const result = spawnSync(path.join(runtime, `${binary}.exe`), args, {
    cwd: temporary, encoding: binaryOutput ? undefined : "utf8", windowsHide: true,
    timeout: 30_000, maxBuffer: 2 * 1024 * 1024,
    env: { ...process.env, PATH: path.join(process.env.SystemRoot ?? "C:\\Windows", "System32") },
  });
  if (result.error) throw result.error;
  assert.equal(result.status, 0, String(result.stderr));
  return result.stdout;
}
try {
  for (const binary of ["ffmpeg", "ffprobe"]) {
    const version = run(binary, ["-version"]);
    assert.ok(version.includes(lock.actualVersion), `${binary} version mismatch`);
    assert.ok(version.includes("--enable-shared") && version.includes("--enable-version3"));
    assert.ok(!version.includes("--enable-gpl") && !version.includes("--enable-nonfree"));
    assert.match(run(binary, ["-L"]), /either version 3 of the License/);
  }
  run("ffmpeg", ["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=5", "-t", "1", "-c:v", "mpeg4", "sample.mp4"]);
  const probe = JSON.parse(run("ffprobe", ["-v", "error", "-show_streams", "-of", "json", "sample.mp4"]));
  assert.equal(probe.streams[0].width, 320);
  assert.equal(probe.streams[0].height, 240);
  run("ffmpeg", ["-v", "error", "-i", "sample.mp4", "-vf", "fps=2,scale=160:120", "frame_%03d.jpg"]);
  assert.equal(fs.readdirSync(temporary).filter(name => /^frame_\d+\.jpg$/.test(name)).length, 2);
  run("ffmpeg", ["-v", "error", "-f", "lavfi", "-i", "color=c=red@0.5:s=64x48:r=1,format=rgba", "-frames:v", "1", "alpha.png"]);
  const rgba = run("ffmpeg", ["-v", "error", "-i", "alpha.png", "-f", "rawvideo", "-pix_fmt", "rgba", "-"], true);
  assert.equal(rgba.length, 64 * 48 * 4);
  assert.ok(rgba[3] === 127 || rgba[3] === 128, "continuous alpha was lost");
  run("ffmpeg", ["-v", "error", "-i", "alpha.png", "-vf", "alphaextract", "-frames:v", "1", "mask.png"]);
  console.log(`Verified ${lock.actualVersion}: executable/DLL pins, LGPLv3, MP4 probe/decode, JPG frames, RGBA and Alpha mask in a Unicode directory.`);
} finally {
  fs.rmSync(temporary, { recursive: true, force: true });
}
