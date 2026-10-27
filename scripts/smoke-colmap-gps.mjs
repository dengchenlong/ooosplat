// Real pinned-engine capability test, using explicitly synthetic geometry.
// This does not substitute for acceptance on real geotagged photographs.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { DatabaseSync } from 'node:sqlite';
import { workspace } from './colmap-runtime.mjs';

const binary = path.resolve(process.argv[2]);
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ooosplat-gps-probe-'));
const fixture = JSON.parse(fs.readFileSync(path.join(workspace, 'scripts/fixtures/colmap-image-io.json'), 'utf8'));
const centers = [[-2,-1,.1],[-1,1,-.2],[1,2,.3],[2,0,-.1],[1,-2,.2],[-1,-2,-.3]];
const points = Array.from({length: 180}, (_, i) => [(i % 15 - 7) * .24, (Math.floor(i/15) - 5) * .22, 5 + (i % 7) * .3]);
const doubles = values => { const b = Buffer.alloc(values.length*8); values.forEach((v,i)=>b.writeDoubleLE(v,i*8)); return b; };
const floats = values => { const b = Buffer.alloc(values.length*4); values.forEach((v,i)=>b.writeFloatLE(v,i*4)); return b; };
function run(args, cwd) {
  const r = spawnSync(binary, args, {cwd, encoding:'utf8', windowsHide:true, timeout:120000, maxBuffer:8*1024*1024});
  fs.appendFileSync(path.join(root,'engine.log'), `${args[0]}\n${r.stdout ?? ''}\n${r.stderr ?? ''}\n`);
  if (r.error || r.status !== 0) throw new Error(`${args[0]} failed (${r.status}): ${r.error ?? r.stderr}`);
}
try {
  for (const model of ['SIMPLE_PINHOLE','EQUIRECTANGULAR']) {
    const dir = path.join(root, model);
    for (const name of ['images','input','output','text']) fs.mkdirSync(path.join(dir,name),{recursive:true});
    centers.forEach((_,i)=>fs.writeFileSync(path.join(dir,'images',`frame_${i+1}.jpg`), Buffer.from(fixture.jpeg,'base64')));
    run(['feature_extractor','--database_path','database.db','--image_path','images','--ImageReader.camera_model',model,
      '--ImageReader.single_camera','1','--FeatureExtraction.use_gpu','0','--FeatureExtraction.num_threads','2'],dir);
    const db = new DatabaseSync(path.join(dir,'database.db'));
    const images = db.prepare('SELECT image_id,name,camera_id FROM images ORDER BY name').all();
    assert.equal(images.length,centers.length);
    const cameraId = images[0].camera_id;
    db.prepare('UPDATE cameras SET width=128,height=?,params=? WHERE camera_id=?')
      .run(model==='EQUIRECTANGULAR'?64:128,doubles(model==='EQUIRECTANGULAR'?[128,64]:[64,64,64]),cameraId);
    const project = (p,c) => {
      const [x,y,z] = p.map((v,j)=>v-c[j]);
      return model === 'EQUIRECTANGULAR' ? [128*(Math.atan2(x,z)/(2*Math.PI)+.5),64*(Math.atan2(y,Math.hypot(x,z))/Math.PI+.5)] : [64*x/z+64,64*y/z+64];
    };
    const descriptorType = db.prepare('SELECT type FROM descriptors LIMIT 1').get().type;
    db.exec('DELETE FROM keypoints; DELETE FROM descriptors; DELETE FROM pose_priors; DELETE FROM two_view_geometries;');
    for (const [i,image] of images.entries()) {
      db.prepare('INSERT INTO keypoints(image_id,rows,cols,data) VALUES(?,?,2,?)').run(image.image_id,points.length,floats(points.flatMap(p=>project(p,centers[i]))));
      db.prepare('INSERT INTO descriptors(image_id,type,rows,cols,data) VALUES(?,?,?,128,?)').run(image.image_id,descriptorType,points.length,Buffer.alloc(points.length*128));
      // Cartesian ENU-like metric target, deliberately 10x the input SfM scale.
      db.prepare('INSERT INTO pose_priors(corr_data_id,corr_sensor_id,corr_sensor_type,position,position_covariance,coordinate_system) VALUES(?,?,0,?,?,1)')
        .run(image.image_id,image.camera_id,doubles(centers[i].map(v=>v*10)),doubles([.01,0,0,0,.01,0,0,0,.01]));
    }
    for(let a=0;a<images.length;a++) for(let b=a+1;b<images.length;b++) {
      const matches=Buffer.alloc(points.length*8);
      for(let i=0;i<points.length;i++){matches.writeUInt32LE(i,i*8);matches.writeUInt32LE(i,i*8+4);}
      const pairId=2147483647*Math.min(images[a].image_id,images[b].image_id)+Math.max(images[a].image_id,images[b].image_id);
      db.prepare('INSERT INTO two_view_geometries(pair_id,rows,cols,data,config) VALUES(?,?,2,?,2)').run(pairId,points.length,matches);
    }
    db.close();
    const dimensions = model==='EQUIRECTANGULAR' ? '128 64 128 64' : '128 128 64 64 64';
    fs.writeFileSync(path.join(dir,'input/cameras.txt'),`${cameraId} ${model} ${dimensions}\n`);
    fs.writeFileSync(path.join(dir,'input/images.txt'),images.map((image,i)=>`${image.image_id} 1 0 0 0 ${centers[i].map(v=>-v).join(' ')} ${cameraId} ${image.name}\n${points.map((p,j)=>`${project(p,centers[i]).join(' ')} ${j+1}`).join(' ')}\n`).join(''));
    fs.writeFileSync(path.join(dir,'input/points3D.txt'),points.map((p,j)=>`${j+1} ${p.join(' ')} 128 128 128 0 ${images.map(image=>`${image.image_id} ${j}`).join(' ')}\n`).join(''));
    run(['pose_prior_mapper','--database_path','database.db','--image_path','images','--input_path','input','--output_path','output',
      '--Mapper.ba_local_backend','CERES','--Mapper.ba_global_backend','CERES','--Mapper.ba_refine_focal_length','0',
      '--Mapper.ba_refine_extra_params','0','--Mapper.num_threads','2','--use_robust_loss_on_prior_position','1'],dir);
    run(['model_converter','--input_path','output','--output_path','text','--output_type','TXT'],dir);
    const rows=fs.readFileSync(path.join(dir,'text/images.txt'),'utf8').split(/\r?\n/).filter(line=>line.includes('frame_'));
    assert.equal(rows.length,centers.length);
    for(const row of rows){
      const f=row.split(/\s+/); const index=Number(/frame_(\d+)/.exec(row)[1])-1;
      // Orientation may rotate during optimization: recover C=-R^T t.
      const [w,x,y,z]=f.slice(1,5).map(Number), t=f.slice(5,8).map(Number);
      const r=[[1-2*(y*y+z*z),2*(x*y-z*w),2*(x*z+y*w)],[2*(x*y+z*w),1-2*(x*x+z*z),2*(y*z-x*w)],[2*(x*z-y*w),2*(y*z+x*w),1-2*(x*x+y*y)]];
      const c=[0,1,2].map(j=>-r.reduce((sum,row,k)=>sum+row[j]*t[k],0));
      assert.ok(Math.hypot(...c.map((v,j)=>v-centers[index][j]*10))<.1,`${model}: metric prior was not applied`);
    }
    console.log(`PASS ${model}: real pose_prior_mapper, Ceres BA, six position priors and metric scale.`);
  }
  console.log(`Synthetic capability evidence: ${root}`);
} catch(error) { console.error(`Evidence retained: ${root}`); throw error; }
