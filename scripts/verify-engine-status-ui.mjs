// Opt-in real browser interaction/layout check. Does not validate native Tauri IPC.
import { build } from 'vite';
import { createServer } from 'node:http';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const out = path.join(root, '.cache/engine-status-ui', String(Date.now()));
await mkdir(out, { recursive: true });
await build({ root, logLevel: 'warn', define: { 'process.env.NODE_ENV': '"production"' }, build: {
  outDir: out, emptyOutDir: false, lib: { entry: path.join(root, 'scripts/fixtures/engine-status.tsx'), name: 'EngineStatusFixture', formats: ['iife'], fileName: () => 'runtime.js', cssFileName: 'style' },
} });
const server = createServer(async (req, res) => {
  try {
    const pathname = new URL(req.url, 'http://localhost').pathname;
    if (pathname === '/') {
      res.setHeader('Content-Type', 'text/html; charset=utf-8');
      res.end('<meta charset="utf-8"><link rel="stylesheet" href="/style.css"><div id="root"></div><script src="/runtime.js"></script>');
    } else if (['/runtime.js', '/style.css'].includes(pathname)) {
      res.setHeader('Content-Type', pathname.endsWith('.js') ? 'application/javascript' : 'text/css');
      res.end(await readFile(path.join(out, pathname.slice(1))));
    } else { res.statusCode = 404; res.end(); }
  } catch (error) { res.statusCode = 500; res.end(String(error)); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const browserPath = process.env.OOOSPLAT_UI_TEST_BROWSER || 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe';
const browser = spawn(browserPath, ['--headless=new', '--no-first-run', '--disable-background-networking', '--disable-sync', '--disable-extensions', '--remote-debugging-port=0', `--user-data-dir=${path.join(out, 'profile')}`, 'about:blank'], { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
let diagnostics = '', socket;
let started, startupFailed;
const startup = new Promise((resolve, reject) => { started = resolve; startupFailed = reject; });
browser.on('error', startupFailed);
browser.on('exit', code => startupFailed(Error(`Browser exited ${code}`)));
browser.stderr.on('data', data => {
  diagnostics += data.toString();
  const port = diagnostics.match(/DevTools listening on ws:\/\/127\.0\.0\.1:(\d+)/)?.[1];
  if (port) started(Number(port));
});
const startupTimeout = setTimeout(() => startupFailed(Error('Browser startup timed out')), 30_000);
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const proofs = [], errors = [];
try {
  const port = await startup;
  const pages = await (await fetch(`http://127.0.0.1:${port}/json/list`, { signal: AbortSignal.timeout(5000) })).json();
  socket = new WebSocket(pages.find(page => page.type === 'page').webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.addEventListener('open', resolve, { once: true }); socket.addEventListener('error', reject, { once: true }); });
  let serial = 0;
  socket.addEventListener('message', event => {
    const message = JSON.parse(event.data);
    if (message.method === 'Runtime.exceptionThrown') errors.push(message.params);
  });
  const call = (method, params = {}) => new Promise((resolve, reject) => {
    const id = ++serial;
    const timer = setTimeout(() => { socket.removeEventListener('message', receive); reject(Error(`${method} timed out`)); }, 15_000);
    const receive = event => {
      const message = JSON.parse(event.data); if (message.id !== id) return;
      clearTimeout(timer); socket.removeEventListener('message', receive);
      message.error ? reject(Error(JSON.stringify(message.error))) : resolve(message.result);
    };
    socket.addEventListener('message', receive); socket.send(JSON.stringify({ id, method, params }));
  });
  const evaluate = async expression => {
    const result = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (result.exceptionDetails) throw Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
  };
  const waitFor = async expression => {
    for (let n = 0; n < 80; n++) { if (await evaluate(expression)) return; await pause(50); }
    throw Error(`Condition timed out: ${expression}`);
  };
  const moveTo = async selector => {
    const point = await evaluate(`(()=>{const r=document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+Math.min(16,r.height/2)};})()`);
    await call('Input.dispatchMouseEvent', { type: 'mouseMoved', ...point });
  };
  await call('Runtime.enable');
  for (const locale of ['zh-CN', 'en']) for (const scale of [80, 100, 140]) {
    await call('Emulation.setDeviceMetricsOverride', { width: 1100, height: 680, deviceScaleFactor: 1, mobile: false });
    await call('Page.navigate', { url: `http://127.0.0.1:${server.address().port}/?locale=${locale}&scale=${scale}` });
    await waitFor('!!document.querySelector(".engine-summary") && typeof window.setEngineStatusFixture === "function"');
    await pause(100); // Allow the app's initial no-IPC engine check to settle before fixture injection.
    for (const state of ['ready', 'issues', 'checking']) {
      await call('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 10, y: 300 });
      await evaluate(`document.activeElement?.blur();window.setEngineStatusFixture(${JSON.stringify(state)});`);
      await pause(250);
      assert.equal(await evaluate('!!document.querySelector(".engine-diagnostics-panel")'), false);
      await moveTo('.engine-summary');
      await waitFor('!!document.querySelector(".engine-diagnostics-panel")');
      const proof = await evaluate(`(()=>{
        const p=document.querySelector('.engine-diagnostics-panel'),r=p.getBoundingClientRect();
        return {summary:document.querySelector('.engine-summary').textContent,text:p.textContent,
          left:r.left,right:r.right,bottom:r.bottom,width:r.width,height:r.height,
          overflow:p.scrollWidth>p.clientWidth+1,scrollable:p.scrollHeight>p.clientHeight,
          selectable:getComputedStyle(p).userSelect,scale:getComputedStyle(document.querySelector('.interface-frame')).getPropertyValue('--ui-scale')};
      })()`);
      assert.doesNotMatch(proof.summary, /ffmpeg|ffprobe|colmap|brush/i);
      assert.ok(proof.left >= 11 && proof.right <= 1089 && proof.bottom <= 669, JSON.stringify(proof));
      assert.equal(proof.overflow, false); assert.equal(proof.selectable, 'text');
      if (state !== 'checking') {
        assert.match(proof.text, /FFmpeg/); assert.match(proof.text, /FFprobe/);
        assert.match(proof.text, /COLMAP/); assert.match(proof.text, /Brush/);
        assert.ok(proof.text.includes(String.raw`\\server\share\引擎目录\ffprobe.exe`));
      }
      await moveTo('.engine-diagnostics-panel'); await pause(250);
      await call('Input.dispatchMouseEvent', { type: 'mouseWheel', x: proof.right - 30, y: Math.min(proof.bottom - 20, 400), deltaX: 0, deltaY: 300 });
      await pause(50); assert.ok(await evaluate('!!document.querySelector(".engine-diagnostics-panel")'));
      if (state !== 'checking') {
        await evaluate(`(()=>{const p=document.querySelector('.engine-diagnostics-panel');p.scrollTop=0;const range=document.createRange();range.selectNodeContents(p.querySelector('.engine-item-path dd'));const selection=getSelection();selection.removeAllRanges();selection.addRange(range);})()`);
        assert.match(await evaluate('getSelection().toString()'), /E:\\Project/);
        await evaluate('getSelection().removeAllRanges()');
      }
      const screenshot = await call('Page.captureScreenshot', { format: 'png' });
      await writeFile(path.join(out, `${locale}-${scale}-${state}.png`), Buffer.from(screenshot.data, 'base64'));
      await call('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 });
      await call('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 });
      await pause(250); assert.equal(await evaluate('!!document.querySelector(".engine-diagnostics-panel")'), false);
      proofs.push({ locale, scale, state, ...proof });
    }
    await evaluate('document.querySelector(".engine-summary").focus()');
    await waitFor('!!document.querySelector(".engine-diagnostics-panel")');
    await call('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Tab', code: 'Tab', windowsVirtualKeyCode: 9 });
    await call('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Tab', code: 'Tab', windowsVirtualKeyCode: 9 });
    assert.equal(await evaluate('document.activeElement.classList.contains("engine-diagnostics-panel")'), true);
    await call('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 });
    await call('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 });
    await pause(50);
    assert.equal(await evaluate('document.activeElement.classList.contains("engine-summary") && !document.querySelector(".engine-diagnostics-panel")'), true);
  }
  assert.equal(errors.length, 0, JSON.stringify(errors));
  await writeFile(path.join(out, 'proof.json'), JSON.stringify({ nativeIPC: false, proofs, errors }, null, 2));
  console.log(`Engine status hover/keyboard/layout verified: ${out}`);
} finally {
  clearTimeout(startupTimeout); socket?.close(); browser.kill();
  server.closeAllConnections(); server.close();
  await writeFile(path.join(out, 'browser.log'), diagnostics);
}
