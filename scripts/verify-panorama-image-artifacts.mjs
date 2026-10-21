// Read-only invariant checks on real image-pipeline output; writes evidence to .cache only.
import assert from 'node:assert/strict';
import { readFile,writeFile,mkdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const project=process.argv[2];if(!project)throw Error('Pass a completed diagnostic panorama image project');
const state=JSON.parse(await readFile(path.join(project,'state.json'),'utf8'));
const manifest=JSON.parse(await readFile(path.join(project,'work/panorama-training/manifest.json'),'utf8'));
assert.equal(state.inputType,'images');assert.equal(state.video,null);
const models=state.reconstruction?.selectedModel;
const model=path.join(project,'work/colmap',models??'sparse/0');
const sphereCamera=await readFile(path.join(model,'cameras.bin'));
assert.equal(sphereCamera.readBigUInt64LE(0),1n);assert.equal(sphereCamera.readInt32LE(12),17);
assert.equal(Number(sphereCamera.readBigUInt64LE(16)),state.panorama.sfmWidth);
assert.equal(Number(sphereCamera.readBigUInt64LE(24)),state.panorama.sfmHeight);
function poses(buffer){let offset=8;const images=[];const count=Number(buffer.readBigUInt64LE(0));for(let i=0;i<count;i++){const id=buffer.readUInt32LE(offset);offset+=4;const qvec=[],tvec=[];for(let j=0;j<4;j++,offset+=8)qvec.push(buffer.readDoubleLE(offset));for(let j=0;j<3;j++,offset+=8)tvec.push(buffer.readDoubleLE(offset));const cameraId=buffer.readUInt32LE(offset);offset+=4;const end=buffer.indexOf(0,offset);assert.ok(end>=offset);const name=buffer.toString('utf8',offset,end);offset=end+1;const observations=Number(buffer.readBigUInt64LE(offset));offset+=8+24*observations;images.push({id,qvec,tvec,cameraId,name,observations});}assert.equal(offset,buffer.length);return images;}
function center({qvec:[w,x,y,z],tvec:[a,b,c]}){const r=[1-2*(y*y+z*z),2*(x*y-z*w),2*(x*z+y*w),2*(x*y+z*w),1-2*(x*x+z*z),2*(y*z-x*w),2*(x*z-y*w),2*(y*z+x*w),1-2*(x*x+y*y)];return [0,1,2].map(i=>-(r[i]*a+r[i+3]*b+r[i+6]*c));}
const sphere=poses(await readFile(path.join(model,'images.bin')));const byId=new Map(sphere.map(p=>[p.id,p]));
const derived=poses(await readFile(path.join(project,'work/panorama-training/sparse/0/images.bin')));
assert.equal(derived.length,sphere.length*6);assert.ok(derived.every(p=>p.observations===0));
assert.equal(new Set(derived.map(p=>p.id)).size,derived.length);
let maxCenterDelta=0;
for(const face of manifest.faces){assert.ok(!('timestamp_seconds'in face)&&!('pts'in face));assert.ok(Number.isInteger(face.source_image_index));const expected=center(byId.get(face.erp_image_id)),actual=center(face);for(let i=0;i<3;i++)maxCenterDelta=Math.max(maxCenterDelta,Math.abs(expected[i]-actual[i]));}
assert.ok(maxCenterDelta<1e-9);
const groups=Map.groupBy(manifest.faces,f=>f.source_image_index);
for(const faces of groups.values())assert.deepEqual(faces.map(f=>f.face).sort(),['back','down','front','left','right','up']);
const oldPoints=await readFile(path.join(model,'points3D.bin'));const newPoints=await readFile(path.join(project,'work/panorama-training/sparse/0/points3D.bin'));
const pointCount=Number(oldPoints.readBigUInt64LE(0));assert.equal(Number(newPoints.readBigUInt64LE(0)),pointCount);
let a=8,b=8;for(let i=0;i<pointCount;i++){assert.ok(oldPoints.subarray(a,a+43).equals(newPoints.subarray(b,b+43)));a+=43;b+=43;const tracks=Number(oldPoints.readBigUInt64LE(a));assert.equal(newPoints.readBigUInt64LE(b),0n);a+=8+tracks*8;b+=8;}assert.equal(a,oldPoints.length);assert.equal(b,newPoints.length);
const proof={project,sourceImages:state.imageSequence.imageCount,registered:sphere.length,faces:derived.length,initializationPoints:pointCount,maxCenterDelta,videoTimestamps:false,oldObservations:false,oldTracks:false};
const out=path.join(root,'.cache/panorama-images-v1');await mkdir(out,{recursive:true});await writeFile(path.join(out,'artifact-proof.json'),JSON.stringify(proof,null,2));console.log(JSON.stringify(proof,null,2));
