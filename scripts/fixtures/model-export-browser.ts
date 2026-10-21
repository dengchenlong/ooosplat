// Real production encoder, with loopback binary transport instead of Tauri IPC.
import type { ModelFormat } from "../../src/lib/modelExportTypes";
declare global { interface Window { fixture: { size: number; count: number; sh: number; formats: Exclude<ModelFormat, "ply">[]; spzAction: "reject" | "accept" | "cancel" } } }
async function convert(format: Exclude<ModelFormat, "ply">) {
  const worker = new Worker(new URL("../../src/workers/gaussianExport.worker.ts", import.meta.url), { type: "module" });
  const started = performance.now();
  let encodingStarted = false, consent = false;
  try {
    await new Promise<void>((resolve, reject) => {
      worker.onerror = e => reject(new Error(e.message));
      worker.onmessage = async e => {
        const m = e.data;
        try {
          if (m.type === "read") {
            const response = await fetch(`/read?format=${format}&file=${m.file}&offset=${m.offset}&length=${m.length}`);
            if (!response.ok) throw new Error(await response.text());
            const bytes = await response.arrayBuffer();
            worker.postMessage({ type: "readReply", exportId: m.exportId, requestId: m.requestId, bytes }, [bytes]);
          } else if (m.type === "confirm") {
            await fetch("/confirmation", { method: "POST", body: JSON.stringify({ format, adjustment: m.adjustment, action: window.fixture.spzAction }) });
            if (window.fixture.spzAction === "cancel") throw new DOMException("Export cancelled", "AbortError");
            if (window.fixture.spzAction !== "accept") throw new Error("SPZ_CONFIRMATION_REQUIRED");
            consent = true;
            worker.postMessage({ type: "confirmReply", exportId: m.exportId, requestId: m.requestId, accepted: true });
          } else if (m.type === "encode") {
            const response = await fetch(`/encode?format=${format}`, { method: "POST" });
            if (!response.ok) throw new Error(await response.text());
            encodingStarted = true;
            worker.postMessage({ type: "encodeReply", exportId: m.exportId, requestId: m.requestId, filename: await response.text() });
          } else if (m.type === "write") {
            if (!encodingStarted) throw new Error("Write before consent/encoding");
            const response = await fetch(`/write?format=${format}&offset=${m.offset}`, { method: "POST", body: m.bytes });
            if (!response.ok) throw new Error(await response.text());
            worker.postMessage({ type: "writeReply", exportId: m.exportId, requestId: m.requestId, written: Number(await response.text()) });
          } else if (m.type === "done") {
            if (!encodingStarted || (m.adjustment && !consent)) throw new Error("Unapproved result");
            await fetch("/result", { method: "POST", body: JSON.stringify({ format, ...m.validation, coordinateAdjustment: m.adjustment, seconds: (performance.now() - started) / 1000 }) });
            resolve();
          } else if (m.type === "error") reject(new Error(m.message));
        } catch (error) { reject(error); }
      };
      worker.postMessage({ type: "start", exportId: `smoke-${format}`, format, sourceSize: window.fixture.size, splatCount: window.fixture.count, shBands: window.fixture.sh });
    });
  } finally { worker.terminate(); }
}
void (async () => {
  try { for (const format of window.fixture.formats) await convert(format); await fetch("/done", { method: "POST" }); }
  catch (error) { await fetch("/done", { method: "POST", body: String(error) }); }
})();
