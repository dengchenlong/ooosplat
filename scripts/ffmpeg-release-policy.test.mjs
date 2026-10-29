import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { assertWindowsFfmpegRelease } from "./ffmpeg-release-policy.mjs";

const manifest = JSON.parse(readFileSync(new URL("../engines/manifest.json", import.meta.url), "utf8"));
const locked = manifest.engines.find(engine => engine.name === "FFmpeg / FFprobe");

test("Windows FFmpeg pins a retained month-end archive and all shared libraries", () => {
  assert.doesNotThrow(() => assertWindowsFfmpegRelease(locked));
  for (const file of ["ffmpeg.exe", "ffprobe.exe", "avcodec-62.dll", "avdevice-62.dll", "avfilter-11.dll", "avformat-62.dll", "avutil-60.dll", "swresample-6.dll", "swscale-9.dll"]) {
    assert.match(manifest.requiredFiles.find(entry => entry.path === `engines/ffmpeg/${file}`)?.sha256 ?? "", /^[a-f0-9]{64}$/i);
  }
  assert.equal(locked.license.split(";")[0], "LGPL-3.0-or-later");
  assert.deepEqual(locked.licenseFiles, ["licenses/FFmpeg-LGPL-3.0.txt", "licenses/FFmpeg-GPL-3.0.txt"]);
});

test("expiring daily builds, floating URLs, wrong variants and invalid locks are rejected", () => {
  for (const sourceUrl of [
    locked.sourceUrl.replace("2026-09-30", "2026-10-03"),
    locked.sourceUrl.replace("2026-09-30", "2026-09-31"),
    locked.sourceUrl.replace("2026-09-30", "2026-13-31"),
    locked.sourceUrl.replace("autobuild-2026-09-30-13-08", "latest"),
    locked.sourceUrl.replace("lgpl-shared", "gpl-shared"),
    locked.sourceUrl.replace("https:", "http:"),
  ]) assert.throws(() => assertWindowsFfmpegRelease({ ...locked, sourceUrl }));
  assert.throws(() => assertWindowsFfmpegRelease({ ...locked, install: { archiveName: "wrong.zip" } }));
  assert.throws(() => assertWindowsFfmpegRelease({ ...locked, archiveSha256: "" }));
});

test("month-end checks handle February and leap years", () => {
  for (const date of ["2024-02-29", "2026-02-28", "2026-01-31"]) {
    assert.doesNotThrow(() => assertWindowsFfmpegRelease({ ...locked, sourceUrl: locked.sourceUrl.replace("2026-09-30", date) }));
  }
  for (const date of ["2026-02-29", "2024-02-28"]) {
    assert.throws(() => assertWindowsFfmpegRelease({ ...locked, sourceUrl: locked.sourceUrl.replace("2026-09-30", date) }));
  }
});
