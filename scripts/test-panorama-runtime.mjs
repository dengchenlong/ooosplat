// Explicit, offline integration check. Never reads/writes production projects.
import { spawn } from 'node:child_process';
import { mkdir, writeFile, readFile, readdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const input = process.argv[2];
if (!input) throw new Error('Usage: node scripts/test-panorama-runtime.mjs <ERP JPEG directory>');
const work = path.join(root, '.cache', 'panorama-v1', `runtime-${Date.now()}`);
await mkdir(path.join(work, 'frames'), { recursive: true });
await mkdir(path.join(work, 'sparse'), { recursive: true });
async function run(executable, args, label) {
  console.log(`${label}: ${executable} ${args.join(' ')}`);
  const chunks = [];
  const child = spawn(executable, args, { cwd: root, windowsHide: true });
  child.stdout.on('data', b => { chunks.push(b); process.stdout.write(b); });
  child.stderr.on('data', b => { chunks.push(b); process.stdout.write(b); });
  const timeout = setTimeout(() => child.kill(), 15 * 60_000);
  const code = await new Promise((resolve, reject) => { child.once('error', reject); child.once('close', resolve); });
  clearTimeout(timeout);
  await writeFile(path.join(work, `${label}.log`), Buffer.concat(chunks));
  if (code !== 0) throw new Error(`${label} failed (${code}); logs: ${work}`);
}
console.log(`Artifacts: ${work}`);
const entries = (await readdir(input)).filter(n => /\.jpg$/i.test(n)).sort();
if (entries.length < 3) throw new Error('Need at least three ERP images');
// Use concat paths, not a shell; paths with spaces and Unicode remain intact.
await writeFile(path.join(work, 'images.txt'), entries.map(n => `file '${path.join(input, n).replaceAll('\\', '/').replaceAll("'", "'\\''")}'\nduration 0.1`).join('\n'));
const ffmpeg = path.join(root, 'engines', 'ffmpeg', 'ffmpeg.exe');
const colmap = path.join(root, 'engines', 'colmap', 'bin', 'colmap.exe');
await run(ffmpeg, ['-hide_banner', '-y', '-f', 'concat', '-safe', '0', '-i', path.join(work, 'images.txt'), '-vf', 'fps=10,scale=1920:960', '-c:v', 'mpeg4', '-q:v', '2', '-pix_fmt', 'yuv420p', path.join(work, 'ballintoy-erp.mp4')], 'encode');
await run(ffmpeg, ['-hide_banner', '-y', '-i', path.join(work, 'ballintoy-erp.mp4'), '-vf', 'fps=4,scale=1536:768', '-q:v', '2', path.join(work, 'frames', '%06d.jpg')], 'frames');
const db = path.join(work, 'database.db');
await run(colmap, ['feature_extractor', '--database_path', db, '--image_path', path.join(work, 'frames'), '--ImageReader.camera_model', 'EQUIRECTANGULAR', '--ImageReader.camera_params', '1536,768', '--ImageReader.single_camera', '1', '--FeatureExtraction.use_gpu', '0', '--FeatureExtraction.num_threads', '8', '--SiftExtraction.max_num_features', '8192'], 'features');
await run(colmap, ['sequential_matcher', '--database_path', db, '--FeatureMatching.use_gpu', '0', '--FeatureMatching.num_threads', '8', '--SequentialMatching.overlap', '10', '--SequentialMatching.loop_detection', '0'], 'matching');
await run(colmap, ['mapper', '--database_path', db, '--image_path', path.join(work, 'frames'), '--output_path', path.join(work, 'sparse'), '--Mapper.ba_local_backend', 'CERES', '--Mapper.ba_global_backend', 'CERES', '--Mapper.init_min_tri_angle', '1', '--Mapper.init_max_forward_motion', '1', '--Mapper.num_threads', '8'], 'mapping');
const models = await readdir(path.join(work, 'sparse'));
if (!models.length) throw new Error(`No reconstructed model: ${work}`);
const camera=await readFile(path.join(work,'sparse',models[0],'cameras.bin'));
if(camera.readBigUInt64LE(0)!==1n || camera.readInt32LE(12)!==17 || camera.readDoubleLE(32)!==1536 || camera.readDoubleLE(40)!==768) throw new Error('Runtime did not produce the expected spherical camera 17 with width,height parameters');
await run(colmap, ['model_analyzer', '--path', path.join(work, 'sparse', models[0])], 'model');
console.log(`Sphere runtime check passed; inspect model.log and camera model 17. Artifacts: ${work}`);
