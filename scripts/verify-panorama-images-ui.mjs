// Opt-in browser/layout check of production components. Not a native Tauri IPC test.
import { build } from 'vite';
import { createServer } from 'node:http';
import { readFile,writeFile,mkdir } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const locale=process.argv[2]==='en'?'en':'zh-CN';
const out=path.join(root,'.cache/panorama-images-ui',`${Date.now()}-${locale}`);
await mkdir(out,{recursive:true});
await build({root,logLevel:'warn',define:{'process.env.NODE_ENV':'"production"'},build:{outDir:out,emptyOutDir:false,lib:{entry:path.join(root,'scripts/fixtures/panorama-images.tsx'),name:'ImagesFixture',formats:['iife'],fileName:()=> 'runtime.js',cssFileName:'style'}}});
const server=createServer(async(req,res)=>{try{const pathname=new URL(req.url,'http://localhost').pathname;if(pathname==='/'){res.setHeader('Content-Type','text/html; charset=utf-8');res.end('<meta charset="utf-8"><link rel="stylesheet" href="/style.css"><div id="root"></div><script src="/runtime.js"></script>');return;}if(['/runtime.js','/style.css'].includes(pathname)){res.setHeader('Content-Type',pathname.endsWith('js')?'application/javascript':'text/css');res.end(await readFile(path.join(out,pathname.slice(1))));return;}res.statusCode=404;res.end();}catch(error){res.statusCode=500;res.end(String(error));}});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
let ready,reject;
const debuggerReady=new Promise((resolve,fail)=>{ready=resolve;reject=fail;});
const browser=spawn('C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',['--headless=new','--no-first-run','--disable-background-networking','--disable-sync','--disable-extensions','--remote-debugging-port=0',`--user-data-dir=${path.join(out,'profile')}`,'about:blank'],{windowsHide:true,stdio:['ignore','ignore','pipe']});
let diagnostics='';browser.stderr.on('data',data=>{diagnostics+=data.toString();const match=diagnostics.match(/DevTools listening on ws:\/\/127\.0\.0\.1:(\d+)/);if(match)ready(Number(match[1]));});browser.on('error',reject);browser.on('exit',code=>reject(Error(`Browser exited ${code}`)));
const timeout=setTimeout(()=>reject(Error('Browser startup timed out')),45000);
let socket;
try{
  const port=await debuggerReady;const pages=await(await fetch(`http://127.0.0.1:${port}/json/list`,{signal:AbortSignal.timeout(5000)})).json();
  socket=new WebSocket(pages.find(page=>page.type==='page').webSocketDebuggerUrl);await new Promise((resolve,fail)=>{const timer=setTimeout(()=>fail(Error('Debugger connection timed out')),10000);socket.addEventListener('open',()=>{clearTimeout(timer);resolve();},{once:true});socket.addEventListener('error',fail,{once:true});});
  let serial=0;const errors=[];
  socket.addEventListener('message',event=>{const message=JSON.parse(event.data);if(message.method==='Runtime.exceptionThrown')errors.push(message.params);});
  const call=(method,params)=>new Promise((resolve,fail)=>{const id=++serial;const timer=setTimeout(()=>{socket.removeEventListener('message',receive);fail(Error(method+' timed out; '+JSON.stringify(errors)));},20000);const receive=event=>{const message=JSON.parse(event.data);if(message.id!==id)return;clearTimeout(timer);socket.removeEventListener('message',receive);if(message.error)fail(Error(JSON.stringify(message.error)));else resolve(message.result);};socket.addEventListener('message',receive);socket.send(JSON.stringify({id,method,params}));});
  await call('Runtime.enable');await call('Emulation.setDeviceMetricsOverride',{width:1100,height:1050,deviceScaleFactor:1,mobile:false});
  await call('Page.navigate',{url:`http://127.0.0.1:${server.address().port}/?locale=${locale}`});
  const check=await call('Runtime.evaluate',{expression:`new Promise((resolve,reject)=>{setTimeout(()=>reject(Error(document.body.innerText)),10000);const wait=()=>document.querySelectorAll('.media-source-status').length===5?requestAnimationFrame(()=>resolve(true)):setTimeout(wait,50);wait();})`,awaitPromise:true,returnByValue:true});
  if(!check.result.value)throw Error('Fixture failed to render');
  const proofs=[];
  for(const zoom of [1,1.4]){
    const proof=await call('Runtime.evaluate',{expression:`(()=>{document.body.style.zoom=${zoom};return [...document.querySelectorAll('.source-input-card,.media-source-status,select')].map(node=>({text:node.textContent,overflow:node.scrollWidth>node.clientWidth+1}));})()`,returnByValue:true});
    proofs.push({zoom,items:proof.result.value});
    if(proof.result.value.some(item=>item.overflow))throw Error('Overflow at zoom '+zoom);
    const screenshot=await call('Page.captureScreenshot',{format:'png'});await writeFile(path.join(out,`source-${zoom}.png`),Buffer.from(screenshot.data,'base64'));
  }
  await call('Runtime.evaluate',{expression:`(()=>{const s=document.querySelector('select');s.value='equirectangular';s.dispatchEvent(new Event('change',{bubbles:true}));s.focus();})()`});
  if(errors.length)throw Error(JSON.stringify(errors));
  await writeFile(path.join(out,'proof.json'),JSON.stringify({locale,errors,proofs,nativeIPC:false},null,2));console.log(`Production image-source UI verified: ${out}`);
}finally{clearTimeout(timeout);socket?.close();browser.kill();server.closeAllConnections();server.close();await writeFile(path.join(out,'browser.log'),diagnostics);}
