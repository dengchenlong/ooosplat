// Real PlayCanvas WebGL2 screenshots at identical views; test output only.
// Usage: node scripts/render-model-export.mjs reference.ply decoded.ply [...]
import { build } from "vite";
import { createServer } from "node:http";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import path from "node:path";
import { spawn } from "node:child_process";
const files = process.argv.slice(2).map(f => path.resolve(f));
const cameraDocument = process.env.CAMERA_POSES_JSON ? JSON.parse(await readFile(process.env.CAMERA_POSES_JSON, "utf8")) : null;
const poses = cameraDocument?.faces?.slice(0, 3) ?? null;
const adjustments = process.env.MODEL_ADJUSTMENTS_JSON ? JSON.parse(await readFile(process.env.MODEL_ADJUSTMENTS_JSON, "utf8")) : null;
if (files.length < 2) throw new Error("Provide a reference PLY and at least one round-trip PLY");
const out = path.resolve(".cache", `model-render-${Date.now()}`); await mkdir(out, { recursive: true });
await build({ configFile: false, root: process.cwd(), logLevel: "warn", build: { target: "es2022", outDir: path.join(out, "bundle"), emptyOutDir: true, rollupOptions: { input: path.resolve("scripts/fixtures/model-export-render.ts"), output: { entryFileNames: "runtime.js" } } } });
let finish; const done = new Promise(resolve => { finish = resolve; });
const server = createServer(async (req, res) => {
  try {
    const pathname = new URL(req.url, "http://127.0.0.1").pathname;
    if (pathname === "/") { res.setHeader("Content-Type", "text/html"); res.end('<script type="module" src="/runtime.js"></script>'); }
    else if (pathname === "/config") res.end(JSON.stringify({ count: files.length, poses, adjustments }));
    else if (/^\/model\/\d+$/.test(pathname)) { const file = files[Number(pathname.split("/").at(-1))]; if (!file) throw new Error("Unknown model"); res.end(await readFile(file)); }
    else if (/^\/capture\/\d+-\d+\.png$/.test(pathname)) { const bytes = []; for await (const chunk of req) bytes.push(chunk); await writeFile(path.join(out, path.basename(pathname)), Buffer.concat(bytes)); res.end("ok"); }
    else if (pathname === "/done") { let body = ""; for await (const b of req) body += b; res.end("ok"); finish(JSON.parse(body)); }
    else { const file = path.resolve(out, "bundle", `.${pathname}`); if (!file.startsWith(path.join(out, "bundle") + path.sep)) throw new Error("Invalid asset"); res.setHeader("Content-Type", "text/javascript"); res.end(await readFile(file)); }
  } catch (error) { res.statusCode = 500; res.end(String(error)); }
});
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
const browser = spawn("C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe", ["--headless=new", "--no-first-run", "--disable-background-networking", "--disable-extensions", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", `--user-data-dir=${path.join(out, "profile")}`, `http://127.0.0.1:${server.address().port}`], { windowsHide: true, stdio: ["ignore", "ignore", "pipe"] });
let diagnostics = ""; browser.stderr.on("data", bytes => { diagnostics = (diagnostics + bytes.toString()).slice(-4096); });
const timer = setTimeout(() => finish({ error: "Browser render timeout" }), 120000);
browser.on("error", e => finish({ error: String(e) }));
browser.on("exit", (code, signal) => finish({ error: `Browser exited before rendering completed (${code}/${signal}): ${diagnostics}` }));
try { const result = await done; await writeFile(path.join(out, "render.json"), JSON.stringify(result, null, 2)); console.log(out, JSON.stringify(result)); if (result.error) throw new Error(result.error); }
finally { clearTimeout(timer); browser.kill(); server.closeAllConnections(); server.close(); }
