// Real production encoder, with loopback binary transport instead of Tauri IPC.
import type { ModelFormat } from "../../src/lib/modelExportTypes";
declare global { interface Window { fixture: { size: number; count: number; sh: number; formats: Exclude<ModelFormat, "ply">[] } } }
async function convert(format: Exclude<ModelFormat, "ply">) {
  const worker = new Worker(new URL("../../src/workers/gaussianExport.worker.ts", import.meta.url), { type: "module" });
  const started = performance.now();
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
            worker.postMessage({ type: "readReply", requestId: m.requestId, bytes }, [bytes]);
          } else if (m.type === "write") {
            const response = await fetch(`/write?format=${format}&offset=${m.offset}`, { method: "POST", body: m.bytes });
            if (!response.ok) throw new Error(await response.text());
            worker.postMessage({ type: "writeReply", requestId: m.requestId, written: Number(await response.text()) });
          } else if (m.type === "done") {
            await fetch("/result", { method: "POST", body: JSON.stringify({ format, ...m.validation, seconds: (performance.now() - started) / 1000 }) });
            resolve();
          } else if (m.type === "error") reject(new Error(m.message));
        } catch (error) { reject(error); }
      };
      worker.postMessage({ type: "start", exportId: `smoke-${format}`, format, filename: `edit.${format === "compressedPly" ? "compressed.ply" : format}`, sourceSize: window.fixture.size, splatCount: window.fixture.count, shBands: window.fixture.sh });
    });
  } finally { worker.terminate(); }
}
void (async () => {
  try { for (const format of window.fixture.formats) await convert(format); await fetch("/done", { method: "POST" }); }
  catch (error) { await fetch("/done", { method: "POST", body: String(error) }); }
})();
