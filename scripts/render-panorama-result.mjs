// Local-only, opt-in Windows visual check using the installed Edge and actual PLY.
import { build } from 'vite';
import { createServer } from 'node:http';
import { createReadStream } from 'node:fs';
import { mkdir, writeFile } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const project=process.argv[2];
if (!project) throw new Error('Usage: node scripts/render-panorama-result.mjs <diagnostic project>');
const out=path.join(root,'.cache/panorama-v1',`render-${Date.now()}`);
await mkdir(out,{recursive:true});
await build({configFile:false,root,logLevel:'warn',define:{'import.meta.url':'location.href'},build:{target:'es2022',outDir:out,emptyOutDir:false,lib:{entry:path.join(root,'scripts/fixtures/panorama-render.ts'),name:'PanoCapture',formats:['iife'],fileName:()=> 'runtime.js'},minify:true}});
let complete, failed;
const done=new Promise((resolve,reject)=>{complete=resolve;failed=reject;});
const server=createServer(async(req,res)=>{
  console.log(`${req.method} ${req.url}`);
  try {
    if(req.url==='/') {res.setHeader('Content-Type','text/html');res.end('<body style="margin:0"><canvas width="480" height="480"></canvas><script src="/runtime.js"></script>');return;}
    if(['/runtime.js','/model.ply','/manifest.json'].includes(req.url)) {
      res.setHeader('Content-Type',req.url.endsWith('.js')?'text/javascript':req.url.endsWith('.json')?'application/json':'application/octet-stream');
      const file=req.url==='/runtime.js'?path.join(out,'runtime.js'):req.url==='/model.ply'?path.join(project,'final.ply'):path.join(project,'work/panorama-training/manifest.json');
      createReadStream(file).pipe(res);return;
    }
    if(/^\/capture\/(front|back|left|right|up|down)$/.test(req.url) && req.method==='POST') {
      const chunks=[];let bytes=0;for await(const b of req){bytes+=b.length;if(bytes>8*1024*1024)throw new Error('Capture oversized');chunks.push(b);}
      await writeFile(path.join(out,`${req.url.split('/').at(-1)}.png`),Buffer.concat(chunks));res.end('ok');return;
    }
    if(req.url==='/done'){res.end('ok');complete();return;}
    if(req.url==='/failed'){const chunks=[];for await(const b of req)chunks.push(b);res.end('error');failed(new Error(Buffer.concat(chunks).toString()));return;}
    res.statusCode=404;res.end();
  }catch(e){res.statusCode=500;res.end(String(e));failed(e);}
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
console.log(`Rendering at http://127.0.0.1:${server.address().port}; artifacts: ${out}`);
const browser=spawn('C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',['--headless=new','--no-first-run','--disable-background-networking','--disable-sync','--disable-extensions','--use-angle=swiftshader','--enable-unsafe-swiftshader',`--user-data-dir=${path.join(out,'profile')}`,`http://127.0.0.1:${server.address().port}`],{windowsHide:true,stdio:['ignore','ignore','pipe']});
browser.stderr.on('data',b=>process.stderr.write(b));
browser.once('error',failed);
const timer=setTimeout(()=>failed(new Error('Browser render timeout')),120000);
try {await done;console.log(`Six actual PLY views saved: ${out}`);}
finally {clearTimeout(timer);browser.kill();server.closeAllConnections();server.close();}
