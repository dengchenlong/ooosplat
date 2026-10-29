import { Application, Asset, Color, Entity, Vec3, Quat, BoundingBox, createGraphicsDevice, DEVICETYPE_WEBGL2, GAMMA_SRGB, TONEMAP_LINEAR, type GSplatResource } from "playcanvas";
import { PreviewSortCoordinator } from "../../src/components/GaussianViewer/PreviewSortCoordinator";
import { configurePreviewRenderer } from "../../src/components/GaussianViewer/PreviewBackend";
import { robustEffectBounds } from "../../src/components/GaussianViewer/PreviewAnimation";

void (async () => {
  try {
    const { count, poses, adjustments } = await (await fetch("/config")).json();
    const canvas = document.createElement("canvas"); canvas.width = 640; canvas.height = 480; document.body.append(canvas);
    const device = await createGraphicsDevice(canvas, { deviceTypes: [DEVICETYPE_WEBGL2], antialias: false, preserveDrawingBuffer: true });
    const app = new Application(canvas, { graphicsDevice: device });
    app.scene.gsplatCentersEnabled = true; configurePreviewRenderer(app); app.scene.gsplat.colorUpdateAngle = 0;
    const sorting = new PreviewSortCoordinator(app), signal = new AbortController().signal;
    const camera = new Entity("Camera"); camera.addComponent("camera", { clearColor: new Color(.055, .067, .09), fov: 55, nearClip: .01, farClip: 1000000, gammaCorrection: GAMMA_SRGB, toneMapping: TONEMAP_LINEAR }); app.root.addChild(camera); app.start();
    const capture = document.createElement("canvas"); capture.width = 640; capture.height = 480;
    const ctx = capture.getContext("2d", { willReadFrequently: true })!;
    const baseline: Uint8ClampedArray[] = []; const results = [];
    let center = new Vec3(), radius = 1;
    for (let index = 0; index < count; index++) {
      const asset = new Asset("model", "gsplat", { url: `/model/${index}`, filename: "model.ply" }, { reorder: false });
      app.assets.add(asset); await new Promise<void>((resolve, reject) => { asset.once("load", resolve); asset.once("error", reject); app.assets.load(asset); });
      const splat = new Entity("model"); splat.setLocalEulerAngles(0, 0, 180); app.root.addChild(splat); splat.addComponent("gsplat", { asset, unified: true });
      if (!index) {
        const resource = asset.resource as GSplatResource, aabb = resource.aabb;
        const robust = robustEffectBounds(resource.centers, { center: [aabb.center.x, aabb.center.y, aabb.center.z], halfExtents: [aabb.halfExtents.x, aabb.halfExtents.y, aabb.halfExtents.z] });
        const bounds = new BoundingBox(); bounds.setFromTransformedAabb(new BoundingBox(new Vec3(...robust.center), new Vec3(...robust.halfExtents)), splat.getWorldTransform());
        center = bounds.center.clone(); radius = Math.max(.1, bounds.halfExtents.length());
      }
      for (let view = 0; view < 3; view++) {
        const angle = view * Math.PI * 2 / 3;
        camera.setPosition(center.x + Math.sin(angle) * radius * 1.8, center.y + radius * .5, center.z + Math.cos(angle) * radius * 1.8); camera.lookAt(center);
        if (poses?.[view]) {
          const { qvec: q, tvec: t } = poses[view], inverse = new Quat(q[1], q[2], q[3], q[0]).invert();
          const mapping = new Vec3(-1, -1, 1);
          const position = inverse.transformVector(new Vec3(-t[0], -t[1], -t[2])).mul(mapping);
          const forward = inverse.transformVector(new Vec3(0, 0, 1)).mul(mapping);
          const up = inverse.transformVector(new Vec3(0, -1, 0)).mul(mapping);
          camera.setPosition(position); camera.lookAt(position.clone().add(forward), up); camera.camera!.fov = 92;
        }
        const adjustment = adjustments?.[index];
        if (adjustment) {
          // The fixture applies the same PLY->engine flip to every model.
          const [x, y, z] = adjustment.center, s = adjustment.scale;
          const c = new Vec3(-x, -y, z);
          camera.setPosition(camera.getPosition().clone().sub(c).mulScalar(s));
        }
        camera.camera!.nearClip = .01 * (adjustment?.scale ?? 1);
        camera.camera!.farClip = 1_000_000 * (adjustment?.scale ?? 1);
        await sorting.waitForFrame(camera.camera!, signal, "Render sort timeout", 30000);
        ctx.drawImage(canvas, 0, 0); const pixels = ctx.getImageData(0, 0, 640, 480).data;
        let error = 0, nonBackground = 0;
        for (let p = 0; p < pixels.length; p += 4) {
          if (Math.abs(pixels[p] - 14) + Math.abs(pixels[p + 1] - 17) + Math.abs(pixels[p + 2] - 23) > 10) nonBackground++;
          if (index) for (let c = 0; c < 3; c++) error += Math.abs(pixels[p + c] - baseline[view][p + c]);
        }
        if (!index) baseline[view] = pixels.slice();
        const mae = error / (640 * 480 * 3); if (index && mae > 25) throw new Error(`Render mismatch ${index}/${view}: ${mae}`);
        if (nonBackground < 1000) throw new Error("Empty render");
        results.push({ index, view, mae, nonBackground });
        const blob = await new Promise<Blob>(resolve => capture.toBlob(value => resolve(value!)));
        await fetch(`/capture/${index}-${view}.png`, { method: "POST", body: blob });
      }
      splat.destroy(); app.assets.remove(asset); asset.unload();
    }
    sorting.destroy(); app.destroy();
    await fetch("/done", { method: "POST", body: JSON.stringify({ backend: "WebGL2", results }) });
  } catch (error) { await fetch("/done", { method: "POST", body: JSON.stringify({ error: String(error) }) }); }
})();
