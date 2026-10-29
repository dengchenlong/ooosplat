// BtbN removes daily releases after 14 builds; dated URLs alone are not durable.
// https://github.com/BtbN/FFmpeg-Builds#release-retention-policy
export function assertWindowsFfmpegRelease(engine) {
  const release = /^https:\/\/github\.com\/BtbN\/FFmpeg-Builds\/releases\/download\/autobuild-(\d{4})-(\d{2})-(\d{2})-\d{2}-\d{2}\/(ffmpeg-n8\.1[^/]*-win64-lgpl-shared-8\.1\.zip)$/.exec(engine?.sourceUrl ?? "");
  if (!release) throw new Error("Windows FFmpeg must use a fixed BtbN 8.1 LGPL shared month-end release, not latest.");
  const [, year, month, day, filename] = release;
  const lastDay = new Date(Date.UTC(Number(year), Number(month), 0)).getUTCDate();
  if (Number(month) < 1 || Number(month) > 12 || Number(day) !== lastDay) {
    throw new Error("Windows FFmpeg must use a month-end release retained for two years, not a daily build removed after 14 builds.");
  }
  if (engine.install?.archiveName !== filename) throw new Error("Windows FFmpeg archive name does not match its locked URL.");
  if (!/^[a-f0-9]{64}$/i.test(engine.archiveSha256 ?? "")) throw new Error("Windows FFmpeg archive SHA-256 must be pinned.");
}
