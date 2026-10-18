// Usage: node scripts/smoke-model-export.mjs <standard.ply> [browser.exe]
// Builds and exercises the real browser worker. Outputs stay under .cache/.
import { build } from "vite";
import { createServer } from "node:http";
import { readFile, stat, open, appendFile, writeFile, mkdir } from "node:fs/promises";
import path from "node:path";
import { spawn } from "node:child_process";
import { spzCspPlugin } from "./spz-csp-plugin.mjs";
const root = process.cwd();
let input = path.resolve(process.argv[2]);
const out = path.join(root, ".cache", `model-export-${Date.now()}`);
await mkdir(out, { recursive: true });
const handle = await open(input, "r");
const headerBytes = Buffer.alloc(256 * 1024); await handle.read(headerBytes, 0, headerBytes.length, 0); await handle.close();
const header = headerBytes.toString("utf8").split("end_header")[0];
const fixture = { size: (await stat(input)).size, count: Number(/element vertex (\d+)/.exec(header)?.[1]), sh: ({ 0: 0, 9: 1, 24: 2, 45: 3 })[(header.match(/property float f_rest_/g) ?? []).length] };
// Optional test-only loopback bridge to the actual Rust edited view.
const viewEndpoint=process.env.MODEL_VIEW_ENDPOINT;
if(viewEndpoint){
  const url=new URL(viewEndpoint);if(url.protocol!=="http:"||url.hostname!=="127.0.0.1")throw new Error("View fixture must be loopback-only");
  const response=await fetch(`${viewEndpoint}/info`);if(!response.ok)throw new Error(await response.text());
  const info=await response.json();fixture.size=info.fileSize;fixture.count=info.splatCount;fixture.sh=info.shBands;
}
fixture.formats = (process.env.SMOKE_FORMATS ?? "compressedPly,spz,sog").split(",");
if (!fixture.formats.length || fixture.formats.some(f => !["compressedPly", "spz", "sog"].includes(f))) throw new Error("Invalid smoke formats");
if (!fixture.count || fixture.sh === undefined) throw new Error("Expected standard Gaussian PLY");
// Optional small real-data fixture for quick codec/WASM checks, never used by the app.
if (process.env.SMOKE_POINTS && !viewEndpoint) {
  const count = Math.min(fixture.count, Number(process.env.SMOKE_POINTS));
  const oldHeaderLength = headerBytes.indexOf("end_header") + "end_header".length + 1;
  const stride = (fixture.size - oldHeaderLength) / fixture.count;
  if (!Number.isInteger(count) || count < 1 || !Number.isInteger(stride)) throw new Error("Invalid fixture count/header");
  const source = await open(input, "r");
  const data = Buffer.alloc(count * stride); await source.read(data, 0, data.length, oldHeaderLength); await source.close();
  input = path.join(out, "fixture.ply");
  await writeFile(input, Buffer.concat([Buffer.from(header.replace(/element vertex \d+/, `element vertex ${count}`) + "end_header\n"), data]));
  fixture.count = count; fixture.size = (await stat(input)).size;
}
await build({ configFile: false, root, logLevel: "warn", worker: { plugins: () => [spzCspPlugin()] }, build: { target: "es2022", outDir: path.join(out, "bundle"), emptyOutDir: true, rollupOptions: { input: path.join(root, "scripts/fixtures/model-export-browser.ts"), output: { entryFileNames: "runtime.js" } } } });
const outputs = Object.fromEntries(["compressedPly", "spz", "sog"].map(f => [f, { path: path.join(out, `edit.${f === "compressedPly" ? "compressed.ply" : f}`), written: 0 }]));
for (const v of Object.values(outputs)) await writeFile(v.path, Buffer.alloc(0));
const results = [];
let finish;
const done = new Promise(resolve => { finish = resolve; });
const resources = new Set();
const server = createServer(async (req, res) => {
  try {
    const url = new URL(req.url, "http://127.0.0.1");
    const output = outputs[url.searchParams.get("format")];
    res.setHeader("Content-Security-Policy", "default-src 'self'; script-src 'self' 'nonce-fixture' 'wasm-unsafe-eval'; connect-src 'self'; worker-src 'self' blob:");
    if (url.pathname === "/") { res.setHeader("Content-Type", "text/html"); res.end(`<script nonce="fixture">window.fixture=${JSON.stringify(fixture)}</script><script type="module" src="/runtime.js"></script>`); }
    else if (url.pathname === "/read") {
      const length = Number(url.searchParams.get("length")), offset = Number(url.searchParams.get("offset"));
      if (!output || length < 1 || length > 1_024_000 || offset < 0) throw new Error("Invalid read");
      if(viewEndpoint&&url.searchParams.get("file")==="source"){
        const response=await fetch(`${viewEndpoint}/read?offset=${offset}&length=${length}`);
        if(!response.ok)throw new Error(await response.text());res.end(Buffer.from(await response.arrayBuffer()));return;
      }
      const h = await open(url.searchParams.get("file") === "source" ? input : output.path, "r");
      try { const bytes = Buffer.alloc(length); const { bytesRead } = await h.read(bytes, 0, length, offset); res.end(bytes.subarray(0, bytesRead)); } finally { await h.close(); }
    } else if (url.pathname === "/write") {
      const chunks = []; let size = 0;
      for await (const b of req) { size += b.length; if (size > 1_024_000) throw new Error("Chunk too large"); chunks.push(b); }
      if (!output || Number(url.searchParams.get("offset")) !== output.written) throw new Error("Out of order write");
      await appendFile(output.path, Buffer.concat(chunks)); output.written += size; res.end(String(output.written));
    } else if (url.pathname === "/result" || url.pathname === "/done") {
      let body = ""; for await (const b of req) body += b;
      if (url.pathname === "/result") { const r = JSON.parse(body); r.bytes = outputs[r.format].written; results.push(r); console.log(JSON.stringify(r)); }
      res.end("ok"); if (url.pathname === "/done") finish(body || null);
    } else {
      const target = path.resolve(out, "bundle", `.${url.pathname}`);
      if (!target.startsWith(path.join(out, "bundle") + path.sep)) throw new Error("Invalid asset");
      resources.add(url.pathname);
      res.setHeader("Content-Type", target.endsWith(".wasm") ? "application/wasm" : "text/javascript"); res.end(await readFile(target));
    }
  } catch (error) { res.statusCode = 500; res.end(String(error)); }
});
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
const browser = spawn(process.argv[3] ?? "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe", ["--headless=new", "--no-first-run", "--disable-background-networking", "--disable-sync", "--disable-extensions", `--user-data-dir=${path.join(out, "profile")}`, `http://127.0.0.1:${server.address().port}`], { windowsHide: true, stdio: "ignore" });
const timer = setTimeout(() => finish("Browser conversion timed out"), 10 * 60_000);
browser.on("error", e => finish(String(e)));
try {
  const error = await done;
  await writeFile(path.join(out, "results.json"), JSON.stringify({ input, fixture, results, resources: [...resources], error }, null, 2));
  console.log(out);
  if (error) throw new Error(error);
  if (results.length !== fixture.formats.length || (fixture.formats.includes("sog") && ![...resources].some(p => p.endsWith(".wasm")))) throw new Error("Missing codec or local WASM verification");
} finally { clearTimeout(timer); browser.kill(); server.closeAllConnections(); server.close(); }
