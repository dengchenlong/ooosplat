// Browser-only production-control/layout checks. No model, training or Tauri writes.
import { build } from 'vite';
import { createServer } from 'node:http';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const out = path.join(root, '.cache/preview-controls', String(Date.now()));
await mkdir(out, { recursive:true });
await build({ root, logLevel:'warn', define:{'process.env.NODE_ENV':'"production"'}, build:{outDir:out,emptyOutDir:false,lib:{entry:path.join(root,'scripts/fixtures/preview-controls.tsx'),name:'PreviewControls',formats:['iife'],fileName:()=> 'runtime.js',cssFileName:'style'}} });
const server = createServer(async(req,res) => {
  try {
    const name = new URL(req.url,'http://localhost').pathname;
    if (name === '/') { res.setHeader('Content-Type','text/html; charset=utf-8'); res.end('<meta charset="utf-8"><link rel="stylesheet" href="/style.css"><div id="root"></div><script src="/runtime.js"></script>'); return; }
    if (!['/style.css','/runtime.js'].includes(name)) { res.writeHead(404); res.end(); return; }
    res.setHeader('Content-Type', name.endsWith('.js')?'application/javascript':'text/css'); res.end(await readFile(path.join(out,name.slice(1))));
  } catch (e) { res.writeHead(500); res.end(String(e)); }
});
await new Promise(resolve => server.listen(0,'127.0.0.1',resolve));
let resolvePort, rejectPort;
const ready = new Promise((resolve,reject) => { resolvePort=resolve; rejectPort=reject; });
const browser = spawn('C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe', ['--headless=new','--no-first-run','--disable-background-networking','--disable-sync','--disable-extensions','--remote-debugging-port=0',`--user-data-dir=${path.join(out,'profile')}`,'about:blank'], {windowsHide:true,stdio:['ignore','ignore','pipe']});
let diagnostics='', socket;
browser.stderr.on('data',data => { diagnostics += data.toString(); const match=diagnostics.match(/DevTools listening on ws:\/\/127\.0\.0\.1:(\d+)/); if(match) resolvePort(Number(match[1])); });
browser.on('error',rejectPort); browser.once('exit',code=>rejectPort(Error(`Browser exited ${code}`)));
const timeout=setTimeout(()=>rejectPort(Error('Browser startup timeout')),30000);
const assert=(value,message)=>{if(!value)throw Error(message);};
try {
  const port=await ready;
  const pages=await(await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  socket=new WebSocket(pages.find(p=>p.type==='page').webSocketDebuggerUrl);
  await new Promise((resolve,reject)=>{socket.addEventListener('open',resolve,{once:true});socket.addEventListener('error',reject,{once:true});});
  let serial=0;
  const call=(method,params={})=>new Promise((resolve,reject)=>{
    const id=++serial;
    const timer=setTimeout(()=>{socket.removeEventListener('message',receive);reject(Error(`${method} timeout`));},15000);
    const receive=event=>{const message=JSON.parse(event.data);if(message.id!==id)return;clearTimeout(timer);socket.removeEventListener('message',receive);if(message.error)reject(Error(JSON.stringify(message.error)));else resolve(message.result);};
    socket.addEventListener('message',receive);socket.send(JSON.stringify({id,method,params}));
  });
  const evaluate=async expression=>{
    const result=await call('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});
    if(result.exceptionDetails)throw Error(JSON.stringify(result.exceptionDetails));return result.result.value;
  };
  await call('Emulation.setDeviceMetricsOverride',{width:1800,height:1000,deviceScaleFactor:1,mobile:false});
  const proofs=[];
  for(const locale of ['zh-CN','en']) {
    await call('Page.navigate',{url:`http://127.0.0.1:${server.address().port}/?locale=${locale}`});
    await evaluate(`new Promise((resolve,reject)=>{const start=performance.now();const poll=()=>document.querySelector('.preview-roam-speed input')?resolve(true):performance.now()-start>8000?reject(Error('fixture not ready')):setTimeout(poll,30);poll();})`);
    for(const zoom of [.8,1,1.4]) {
      const proof=await evaluate(`new Promise(resolve=>{document.body.style.zoom=${zoom};requestAnimationFrame(()=>{const header=document.querySelector('.frame-sampling-control .field-label-row'),output=header.querySelector('output'),speed=document.querySelector('.preview-roam-speed'),scroll=document.querySelector('.preview-status-items');const a=header.getBoundingClientRect(),b=output.getBoundingClientRect(),c=speed.getBoundingClientRect();resolve({centerError:Math.abs((a.left+a.right-b.left-b.right)/2),titleColor:getComputedStyle(header.querySelector('label')).color,speedRight:c.right,speedWidth:c.width,viewport:innerWidth,statisticsScroll:scroll.scrollWidth>scroll.clientWidth,oldHint:document.body.innerText.includes('仅首次抽帧，不含自动补帧。'),label:header.querySelector('label').textContent,speedLabel:speed.querySelector('label').textContent});});})`);
      assert(proof.centerError<1,'Percentage is not centered');assert(proof.titleColor==='rgb(127, 137, 153)','Sampling title not gray');
      assert(proof.speedRight<=proof.viewport+1 && proof.speedWidth>100,'Speed control clipped');assert(proof.statisticsScroll,'Statistics did not scroll independently');assert(!proof.oldHint,'Old hint remains');
      proofs.push({locale,zoom,...proof});
      const screenshot=await call('Page.captureScreenshot',{format:'png'});await writeFile(path.join(out,`${locale}-${zoom}.png`),Buffer.from(screenshot.data,'base64'));
    }
    await evaluate(`(()=>{document.body.style.zoom=1;const field=document.querySelector('.transform-field input');field.focus();})()`);
    await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});
    await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});
    assert(await evaluate(`document.activeElement.tagName==='CANVAS'`),'Enter did not return canvas focus');
    await evaluate(`document.querySelector('.preview-roam-speed input').focus()`);
    await call('Input.dispatchKeyEvent',{type:'keyDown',key:'ArrowRight',code:'ArrowRight',windowsVirtualKeyCode:39});
    await call('Input.dispatchKeyEvent',{type:'keyUp',key:'ArrowRight',code:'ArrowRight',windowsVirtualKeyCode:39});
    assert(await evaluate(`document.activeElement.matches('.preview-roam-speed input')`),'Keyboard speed control lost focus');
    await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});
    await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});
    assert(await evaluate(`document.activeElement.tagName==='CANVAS'`),'Speed Enter did not return canvas focus');
  }
  await writeFile(path.join(out,'proof.json'),JSON.stringify({proofs,nativeDesktop:false,modelRendering:false},null,2));
  console.log(JSON.stringify({out,checks:proofs.length,centered:true,statisticsScroll:true,keyboardFocus:true}));
} finally { clearTimeout(timeout);socket?.close();browser.kill();server.closeAllConnections();server.close();await writeFile(path.join(out,'browser.log'),diagnostics); }
