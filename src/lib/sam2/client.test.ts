import { describe, expect, it } from "vitest";
import { SidecarClient } from "./client";
import { SidecarRequestError, type SidecarResponse } from "./protocol";
import { ScriptedTransport } from "./testTransport";

function framesPayload() {
  return [{ frameId: "0000", path: "C:/proj/work/frames/0000.png" }];
}

describe("SidecarClient", () => {
  it("hello() sends the protocol handshake and returns backend info", async () => {
    const t = new ScriptedTransport();
    const client = new SidecarClient(t);
    const pending = client.hello();
    t.push('{"type":"hello","protocolVersion":1,"backend":"synthetic","model":"m","localOnly":true}');
    const hello = await pending;
    expect(hello.backend).toBe("synthetic");
    expect(hello.localOnly).toBe(true);
    expect(t.requestOf(0)).toEqual({ type: "hello", protocolVersion: 1 });
  });

  it("initSession returns the frame count", async () => {
    const t = new ScriptedTransport();
    const client = new SidecarClient(t);
    const pending = client.initSession("s1", framesPayload());
    t.push('{"type":"session_ready","sessionId":"s1","frameCount":1}');
    const ready = await pending;
    expect(ready.frameCount).toBe(1);
    expect(t.requestOf(0)).toEqual({ type: "init_session", sessionId: "s1", frames: framesPayload() });
  });

  it("addPrompt returns the anchor count (multi-anchor re-prompt path)", async () => {
    const t = new ScriptedTransport();
    const client = new SidecarClient(t);
    const pending = client.addPrompt({
      sessionId: "s1",
      objectId: "obj",
      frameId: "0002",
      points: [[10, 10]],
      pointLabels: [1],
    });
    t.push('{"type":"prompt_accepted","sessionId":"s1","objectId":"obj","frameId":"0002","anchorCount":2}');
    const accepted = await pending;
    expect(accepted.anchorCount).toBe(2);
  });

  it("propagate streams mask responses then resolves with propagate_done", async () => {
    const t = new ScriptedTransport();
    const client = new SidecarClient(t);
    const masks: SidecarResponse[] = [];
    const pending = client.propagate(
      { sessionId: "s1", objectId: "obj", masksDir: "C:/masks" },
      { onMask: (m) => masks.push(m) },
    );
    for (const frameId of ["0000", "0001"]) {
      t.push(
        `{"type":"mask","sessionId":"s1","objectId":"obj","frameId":"${frameId}",` +
          `"maskPath":"C:/masks/${frameId}.png","width":64,"height":48,"coverage":0.1}`,
      );
    }
    t.push('{"type":"propagate_done","sessionId":"s1","objectId":"obj","completed":2,"skipped":0}');
    const done = await pending;
    expect(done.completed).toBe(2);
    expect(done.skipped).toBe(0);
    expect(masks.map((m) => (m as { frameId: string }).frameId)).toEqual(["0000", "0001"]);
    expect(t.requestOf(0)).toEqual({ type: "propagate", sessionId: "s1", objectId: "obj", masksDir: "C:/masks" });
  });

  it("converts error responses into SidecarRequestError", async () => {
    const t = new ScriptedTransport();
    const client = new SidecarClient(t);
    const pending = client.initSession("s1", framesPayload());
    t.push('{"type":"error","code":"invalid_request","message":"frame file not found: x.png","inReplyTo":"init_session"}');
    await expect(pending).rejects.toThrow(SidecarRequestError);
    await expect(pending).rejects.toMatchObject({ code: "invalid_request" });
  });

  it("rejects concurrent requests", async () => {
    const t = new ScriptedTransport();
    const client = new SidecarClient(t);
    const first = client.ping();
    await expect(client.ping()).rejects.toThrow(/in flight/i);
    t.push('{"type":"pong"}');
    await first;
  });

  it("shutdown sends the shutdown request and resolves on bye", async () => {
    const t = new ScriptedTransport();
    const client = new SidecarClient(t);
    const pending = client.shutdown();
    t.push('{"type":"bye"}');
    await pending;
    expect(t.requestOf(0)).toEqual({ type: "shutdown" });
  });

  it("aborts propagate via AbortSignal and disposes the transport", async () => {
    const t = new ScriptedTransport();
    const client = new SidecarClient(t);
    const controller = new AbortController();
    const pending = client.propagate(
      { sessionId: "s1", objectId: "obj", masksDir: "C:/masks" },
      { signal: controller.signal },
    );
    t.push('{"type":"mask","sessionId":"s1","objectId":"obj","frameId":"0000","maskPath":"m.png","width":64,"height":48,"coverage":0.1}');
    t.push('{"type":"mask","sessionId":"s1","objectId":"obj","frameId":"0001","maskPath":"m1.png","width":64,"height":48,"coverage":0.1}');
    controller.abort();
    await expect(pending).rejects.toMatchObject({ name: "AbortError" });
    expect(t.disposed).toBe(true);
  });
});
