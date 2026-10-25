// Local opt-in real-renderer verification. Does not change project data or run training.
import { build } from "vite";
import { createServer } from "node:http";
import { createReadStream, createWriteStream } from "node:fs";
import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const [project, referenceFile, renderer = "webgl2", locale = "zh", width = "1600"] = process.argv.slice(2);
if (!project || !referenceFile || !["webgl2", "webgpu"].includes(renderer) || !/^\d+$/.test(width) || +width < 1100 || +width > 3840) throw Error("Usage: node scripts/verify-preview-navigation.mjs <project> <backend-reference.json> [webgl2|webgpu] [zh|en] [window-width>=1100]");
const out = path.join(root, ".cache/preview-navigation", `${Date.now()}-${renderer}-${locale}`);
await mkdir(out, { recursive: true });
await build({ root, define: { "import.meta.url": "location.href", "process.env.NODE_ENV": '"production"' }, logLevel: "warn",
  build: { outDir: out, emptyOutDir: false, lib: { entry: path.join(root, "scripts/fixtures/preview-navigation.tsx"), name: "PreviewNavigation", formats: ["iife"], fileName: () => "runtime.js", cssFileName: "style" } } });
const model = path.join(project, "final.ply");
const info = await stat(model);
let header = "";
for await (const chunk of createReadStream(model, { start: 0, end: 65535 })) header += chunk.toString("utf8");
const count = Number(header.match(/element vertex (\d+)/)?.[1]);
if (!count) throw Error("Missing Gaussian count");
const referenceView = JSON.parse(await readFile(referenceFile, "utf8"));
const descriptor = { projectId: "00000000-0000-4000-8000-000000000001", modelPath: model, assetPath: model, format: "ply", fileSize: info.size, splatCount: count,
  transform: { position: [0, 0, 0], rotation: [0, 0, 0], scale: 1 }, editing: { crop: null, revision: 0, sourceSplatCount: count, deletedCount: 0 }, editMaskAssetPath: null, referenceView };
let complete, fail, capture;
const done = new Promise((resolve, reject) => { complete = resolve; fail = reject; });
const server = createServer(async (req, res) => {
  try {
    const pathname = new URL(req.url, "http://localhost").pathname;
    if (pathname === "/") { res.setHeader("Content-Type", "text/html; charset=utf-8"); res.end('<meta charset="utf-8"><link rel="stylesheet" href="/style.css"><div id="root"></div><script src="/runtime.js"></script>'); return; }
    if (pathname === "/descriptor.json") { res.setHeader("Content-Type", "application/json"); res.end(JSON.stringify(descriptor)); return; }
    if (pathname === "/snapshot") {
      const name = new URL(req.url, "http://localhost").searchParams.get("name");
      if (!capture || !/^[a-z0-9-]{1,40}$/.test(name ?? "")) throw Error("Invalid verification screenshot");
      await capture(name); res.end("ok"); return;
    }
    if (["/runtime.js", "/style.css", "/model.ply"].includes(pathname)) {
      const file = pathname === "/model.ply" ? model : path.join(out, pathname.slice(1));
      res.setHeader("Content-Type", pathname.endsWith(".js") ? "application/javascript" : pathname.endsWith(".css") ? "text/css" : "application/octet-stream");
      const bytes = (await stat(file)).size;
      if (pathname === "/model.ply" && req.headers.range) {
        const range = /^bytes=(\d+)-(\d+)$/.exec(req.headers.range);
        const start = range ? Number(range[1]) : NaN;
        const end = range ? Number(range[2]) : NaN;
        if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || start < 0 || end < start || end >= bytes) {
          res.writeHead(416); res.end(); return;
        }
        res.statusCode = 206;
        res.setHeader("Content-Range", `bytes ${start}-${end}/${bytes}`);
        res.setHeader("Content-Length", end - start + 1);
        createReadStream(file, { start, end }).pipe(res); return;
      }
      res.setHeader("Content-Length", bytes); createReadStream(file).pipe(res); return;
    }
    if (req.method === "POST" && ["/proof", "/failed"].includes(pathname)) {
      const chunks = []; let bytes = 0;
      for await (const chunk of req) { bytes += chunk.length; if (bytes > 1024 * 1024) throw Error("Oversized proof"); chunks.push(chunk); }
      const body = Buffer.concat(chunks).toString("utf8"); res.end("ok");
      if (pathname === "/failed") fail(Error(body)); else complete(JSON.parse(body)); return;
    }
    res.statusCode = 404; res.end();
  } catch (error) { res.statusCode = 500; res.end(String(error)); fail(error); }
});
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
const address = `http://127.0.0.1:${server.address().port}`;
console.log(`Preview verification: ${address}; artifacts: ${out}`);
let debugReady;
const debuggerReady = new Promise(resolve => { debugReady = resolve; });
const browser = spawn("C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe", ["--headless=new", "--no-first-run", "--disable-background-networking", "--disable-sync", "--disable-extensions", `--window-size=${width},1050`, "--remote-debugging-port=0", "--enable-unsafe-webgpu",
  ...(renderer === "webgl2" && process.env.OOOSPLAT_PREVIEW_HARDWARE_GL !== "1" ? ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"] : []), `--user-data-dir=${path.join(out, "profile")}`, "about:blank"], { windowsHide: true, stdio: ["ignore", "ignore", "pipe"] });
browser.on("error", fail);
browser.once("exit", code => fail(Error(`Verification browser exited (${code}) before proof; see ${out}/browser.log`)));
const diagnostics = createWriteStream(path.join(out, "browser.log"));
browser.stderr.on("data", data => { diagnostics.write(data); const text = data.toString(); const match = text.match(/DevTools listening on ws:\/\/127\.0\.0\.1:(\d+)/); if (match) { console.log(`Preview debugger port: ${match[1]}`); debugReady(Number(match[1])); } });
const timer = setTimeout(() => fail(Error("Real preview verification timed out")), 120000);
try {
  const debugPort = await debuggerReady;
  const pages = await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json();
  const page = pages.find(p => p.type === "page");
  if (!page) throw Error("Cannot capture the actual preview page");
  const socket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.addEventListener("open", resolve, { once: true }); socket.addEventListener("error", reject, { once: true }); });
  let requestId = 0;
  const call = (method, params) => new Promise((resolve, reject) => {
    const id = ++requestId;
    const receive = event => {
      const message = JSON.parse(event.data);
      if (message.id !== id) return;
      socket.removeEventListener("message", receive);
      if (message.error) reject(Error(JSON.stringify(message.error))); else resolve(message.result);
    };
    socket.addEventListener("message", receive);
    socket.send(JSON.stringify({ id, method, params }));
  });
  // OS window dimensions include chrome/borders. Set the actual CSS viewport
  // before loading, so the 1100px minimum-width layout is not silently clipped.
  await call("Emulation.setDeviceMetricsOverride", { width: Number(width), height: 1050, deviceScaleFactor: 1, mobile: false });
  capture = async name => {
    const screenshot = await call("Page.captureScreenshot", { format: "png" });
    await writeFile(path.join(out, `${name}.png`), Buffer.from(screenshot.data, "base64"));
  };
  await call("Page.navigate", { url: `${address}/?renderer=${renderer}&locale=${locale}` });
  const proof = await done;
  const screenshot = await call("Page.captureScreenshot", { format: "png" });
  await writeFile(path.join(out, "preview.png"), Buffer.from(screenshot.data, "base64"));
  socket.close();
  await writeFile(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(JSON.stringify({ out, ...proof }, null, 2));
} finally { clearTimeout(timer); browser.kill(); diagnostics.end(); server.closeAllConnections(); server.close(); }
