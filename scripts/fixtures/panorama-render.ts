// Real output visual acceptance only; no production UI or model manipulation.
import { Application, Asset, Color, Entity, Vec3, createGraphicsDevice, DEVICETYPE_WEBGL2, GAMMA_SRGB, TONEMAP_LINEAR } from "playcanvas";
import { configurePreviewRenderer } from "../../src/components/GaussianViewer/PreviewBackend";
import { PreviewSortCoordinator } from "../../src/components/GaussianViewer/PreviewSortCoordinator";

async function capture() {
  const canvas = document.querySelector("canvas")!;
  const device = await createGraphicsDevice(canvas, { deviceTypes: [DEVICETYPE_WEBGL2], antialias: false, preserveDrawingBuffer: true });
  const app = new Application(canvas, { graphicsDevice: device });
  app.scene.gsplatCentersEnabled = true;
  configurePreviewRenderer(app);
  app.scene.gsplat.colorUpdateAngle = 0;
  app.setCanvasResolution("FIXED", 480, 480);
  const sorting = new PreviewSortCoordinator(app);
  const camera = new Entity("Camera");
  camera.addComponent("camera", { fov: 92, nearClip: 0.001, farClip: 10000, gammaCorrection: GAMMA_SRGB, toneMapping: TONEMAP_LINEAR, clearColor: new Color(0,0,0) });
  app.root.addChild(camera);
  const asset = new Asset("model", "gsplat", { url: "/model.ply" }, { reorder: false });
  app.assets.add(asset);
  await new Promise<void>((resolve,reject) => {asset.once("load",resolve);asset.once("error",reject);app.assets.load(asset);});
  const model = new Entity("model");
  // Match the existing OOOSplat preview coordinate mapping.
  model.setLocalEulerAngles(0,0,180);
  model.addComponent("gsplat", {asset,unified:true});app.root.addChild(model);
  const manifest = await (await fetch("/manifest.json")).json();
  app.start();
  const first = manifest.faces[0].erp_image_id;
  for (const face of manifest.faces.filter((f: {erp_image_id:number})=>f.erp_image_id===first)) {
    const [w,x,y,z] = face.qvec;
    const r = [[1-2*(y*y+z*z),2*(x*y-z*w),2*(x*z+y*w)],[2*(x*y+z*w),1-2*(x*x+z*z),2*(y*z-x*w)],[2*(x*z-y*w),2*(y*z+x*w),1-2*(x*x+y*y)]];
    const center = [0,1,2].map(i => -r.reduce((s,row,j)=>s+row[i]*face.tvec[j],0));
    const map = (v:number[]) => new Vec3(-v[0],-v[1],v[2]);
    const position = map(center), forward = map(r[2]), up = map(r[1].map(v=>-v));
    camera.setPosition(position);camera.lookAt(position.clone().add(forward),up);
    await sorting.waitForFrame(camera.camera!,new AbortController().signal,"Capture sorting timeout",30000);
    const blob = await new Promise<Blob>((resolve,reject)=>canvas.toBlob(b=>b?resolve(b):reject(new Error("No PNG"))));
    await fetch(`/capture/${face.face}`, {method:"POST",body:blob});
  }
  sorting.destroy();app.destroy();await fetch("/done",{method:"POST"});
}
void capture().catch(error=>fetch("/failed",{method:"POST",body:String(error)}));
