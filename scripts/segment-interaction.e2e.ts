/**
 * Integration test: first-frame prompt -> sidecar client -> full-frame
 * propagate -> per-frame masks on disk (issue #2 acceptance path, driven by
 * the deterministic synthetic backend instead of the real SAM2 weights).
 *
 * This exercises exactly the JSON-lines conversation the Rust/TS host holds
 * with `python -m sam2_sidecar --backend synthetic`:
 *
 *   hello -> init_session -> add_prompt (points/box)
 *         -> [add_prompt on a later frame = re-anchor] -> propagate
 *         -> mask* -> propagate_done -> shutdown
 *
 * Also verifies the resume contract: a second propagate with the same prompt
 * finds every mask already on disk (completed=0, skipped=N) — the "propagation
 * interrupted, can resume" acceptance item.
 *
 * Standalone (not in vitest): `npx tsx scripts/segment-interaction.e2e.ts`
 */
import { spawn, type ChildProcess } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync, existsSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { SidecarClient } from "../src/lib/sam2/client";
import type { MaskResponse } from "../src/lib/sam2/protocol";
import type { SidecarTransport } from "../src/lib/sam2/transport";
import { toAddPromptRequest } from "../src/lib/sam2/prompt";

const SIDECAR_DIR = join(fileURLToPath(new URL("../engines/sam2/sidecar", import.meta.url)));
const UV = process.platform === "win32" ? "uv.exe" : "uv";

/** Node child-process transport: stdin/stdout JSON lines, stderr drained. */
class NodeTransport implements SidecarTransport {
  private readonly proc: ChildProcess;
  private readonly buffer: string[] = [];
  private waiters: ((line: string | null) => void)[] = [];
  private stdoutText = "";

  constructor(args: string[]) {
    this.proc = spawn(UV, ["run", "python", "-m", "sam2_sidecar", ...args], {
      cwd: SIDECAR_DIR,
      stdio: ["pipe", "pipe", "pipe"],
    });
    this.proc.stdout!.setEncoding("utf-8");
    this.proc.stdout!.on("data", (chunk: string) => this.onData(chunk));
    this.proc.stderr!.setEncoding("utf-8");
    this.proc.stderr!.on("data", (chunk: string) => process.stderr.write(chunk));
    this.proc.on("exit", () => {
      for (const waiter of this.waiters.splice(0)) waiter(null);
    });
  }

  private onData(chunk: string): void {
    this.stdoutText += chunk;
    for (;;) {
      const newline = this.stdoutText.indexOf("\n");
      if (newline === -1) break;
      const line = this.stdoutText.slice(0, newline);
      this.stdoutText = this.stdoutText.slice(newline + 1);
      const waiter = this.waiters.shift();
      if (waiter) waiter(line);
      else this.buffer.push(line);
    }
  }

  writeLine(line: string): void {
    this.proc.stdin!.write(line);
  }

  readLine(): Promise<string | null> {
    const buffered = this.buffer.shift();
    if (buffered !== undefined) return Promise.resolve(buffered);
    return new Promise((resolve) => this.waiters.push(resolve));
  }

  async dispose(): Promise<void> {
    this.proc.kill();
  }
}

function makePng(path: string, width: number, height: number): void {
  // Minimal 8-bit RGB PNG, solid green — content is irrelevant to the
  // synthetic backend, which grows a square around the prompted point.
  const raw = Buffer.alloc(height * (1 + width * 3));
  for (let y = 0; y < height; y++) {
    const row = y * (1 + width * 3);
    raw[row] = 0;
    for (let x = 0; x < width; x++) {
      raw[row + 1 + x * 3] = 10;
      raw[row + 1 + x * 3 + 1] = 200;
      raw[row + 1 + x * 3 + 2] = 30;
    }
  }
  const chunk = (tag: Buffer, data: Buffer): Buffer => {
    const payload = Buffer.concat([tag, data]);
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(payload));
    return Buffer.concat([len, payload, crc]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 2; // color type RGB
  writeFileSync(
    path,
    Buffer.concat([
      Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
      chunk(Buffer.from("IHDR"), ihdr),
      chunk(Buffer.from("IDAT"), zlibSync(raw)),
      chunk(Buffer.from("IEND"), Buffer.alloc(0)),
    ]),
  );
}

// Minimal zlib (stored deflate blocks) + CRC32 — stdlib-free PNG support.
function crc32(buf: Buffer): number {
  let crc = 0xffffffff;
  for (const byte of buf) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) {
      crc = crc & 1 ? (crc >>> 1) ^ 0xedb88320 : crc >>> 1;
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function zlibSync(data: Buffer): Buffer {
  const out: Buffer[] = [];
  const blockSize = 65535;
  for (let offset = 0; offset < data.length; offset += blockSize) {
    const block = data.subarray(offset, Math.min(offset + blockSize, data.length));
    const header = Buffer.alloc(5);
    header[0] = offset + blockSize >= data.length ? 1 : 0; // BFINAL + BTYPE=00
    header.writeUInt16LE(block.length, 1);
    header.writeUInt16LE(~block.length & 0xffff, 3);
    out.push(header, block);
  }
  const checksum = Buffer.alloc(4);
  checksum.writeUInt32BE(adler32(data));
  return Buffer.concat([Buffer.from([0x78, 0x01]), ...out, checksum]);
}

function adler32(data: Buffer): number {
  let a = 1;
  let b = 0;
  for (const byte of data) {
    a = (a + byte) % 65521;
    b = (b + a) % 65521;
  }
  return ((b << 16) | a) >>> 0;
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(`E2E FAIL: ${message}`);
}

async function main(): Promise<number> {
  const workspace = mkdtempSync(join(tmpdir(), "segment-e2e-"));
  const framesDir = join(workspace, "frames");
  const masksDir = join(workspace, "masks");
  mkdirSync(framesDir, { recursive: true });
  const FRAME_COUNT = 6;
  for (let i = 0; i < FRAME_COUNT; i++) {
    makePng(join(framesDir, `${String(i).padStart(4, "0")}.png`), 64, 48);
  }

  const transport = new NodeTransport(["--backend", "synthetic"]);
  const client = new SidecarClient(transport);
  try {
    const hello = await client.hello();
    assert(hello.backend === "synthetic" && hello.localOnly, `hello: ${JSON.stringify(hello)}`);

    const frames = Array.from({ length: FRAME_COUNT }, (_, i) => ({
      frameId: String(i).padStart(4, "0"),
      path: join(framesDir, `${String(i).padStart(4, "0")}.png`),
    }));
    const ready = await client.initSession("e2e", frames);
    assert(ready.frameCount === FRAME_COUNT, `session_ready: ${JSON.stringify(ready)}`);

    // --- first-frame point prompt -------------------------------------
    const firstFramePrompt = {
      objectId: "bracelet",
      anchorFrameId: "0000",
      points: [{ x: 32, y: 24, label: 1 as const }],
      box: null,
    };
    const accepted = await client.addPrompt(toAddPromptRequest(firstFramePrompt, "e2e"));
    assert(accepted.anchorCount === 1, `anchorCount after first prompt: ${JSON.stringify(accepted)}`);

    const masks: MaskResponse[] = [];
    const done = await client.propagate(
      { sessionId: "e2e", objectId: "bracelet", masksDir },
      { onMask: (m) => masks.push(m) },
    );
    assert(
      done.completed === FRAME_COUNT && done.skipped === 0,
      `propagate_done: ${JSON.stringify(done)}`,
    );
    assert(masks.length === FRAME_COUNT, `expected ${FRAME_COUNT} mask responses, got ${masks.length}`);
    for (const mask of masks) {
      assert(existsSync(mask.maskPath), `mask missing on disk: ${mask.maskPath}`);
      const png = readFileSync(mask.maskPath);
      assert(png.subarray(1, 4).toString() === "PNG", `not a PNG: ${mask.maskPath}`);
      assert(mask.coverage > 0 && mask.coverage < 1, `coverage out of range: ${mask.coverage}`);
    }
    console.log(
      `E2E: point prompt -> ${masks.length} masks, coverage ${Math.min(...masks.map((m) => m.coverage)).toFixed(3)}..${Math.max(...masks.map((m) => m.coverage)).toFixed(3)}`,
    );

    // --- re-anchor (multi-anchor re-prompt against loop drift) ---------
    const accepted2 = await client.addPrompt({
      type: "add_prompt",
      sessionId: "e2e",
      objectId: "bracelet",
      frameId: "0003",
      points: [[40, 20]],
      pointLabels: [1],
    });
    assert(accepted2.anchorCount === 2, `anchorCount after re-anchor: ${JSON.stringify(accepted2)}`);
    const masksAfterAnchor: MaskResponse[] = [];
    const done2 = await client.propagate(
      { sessionId: "e2e", objectId: "bracelet", masksDir },
      { onMask: (m) => masksAfterAnchor.push(m) },
    );
    assert(
      done2.completed === FRAME_COUNT && done2.skipped === 0,
      `re-anchored propagate must recompute all frames: ${JSON.stringify(done2)}`,
    );
    console.log(`E2E: re-anchor at 0003 -> anchorCount=2, recomputed ${done2.completed} frames`);

    // --- resume: same prompt again finds everything on disk ------------
    const done3 = await client.propagate({ sessionId: "e2e", objectId: "bracelet", masksDir });
    assert(
      done3.completed === 0 && done3.skipped === FRAME_COUNT,
      `resume must skip existing masks: ${JSON.stringify(done3)}`,
    );
    console.log(`E2E: resume -> completed=0 skipped=${done3.skipped} (checkpointed propagation)`);
  } finally {
    try {
      await client.shutdown();
    } catch {
      await transport.dispose();
    }
  }
  console.log(`E2E OK -> ${workspace}`);
  return 0;
}

main()
  .then((code) => process.exit(code))
  .catch((error) => {
    console.error(error);
    process.exit(1);
  });
