import { describe, expect, it } from "vitest";
import {
  PROTOCOL_VERSION,
  SidecarRequestError,
  decodeSidecarMessage,
  encodeSidecarMessage,
} from "./protocol";

describe("encodeSidecarMessage", () => {
  it("serializes one JSON object per newline-terminated line", () => {
    const line = encodeSidecarMessage({ type: "hello", protocolVersion: PROTOCOL_VERSION });
    expect(line.endsWith("\n")).toBe(true);
    expect(JSON.parse(line)).toEqual({ type: "hello", protocolVersion: 1 });
  });
});

describe("decodeSidecarMessage", () => {
  it("parses a single response line", () => {
    const msg = decodeSidecarMessage('{"type":"pong"}\n');
    expect(msg).toEqual({ type: "pong" });
  });

  it("tolerates a trailing carriage return", () => {
    const msg = decodeSidecarMessage('{"type":"pong"}\r\n');
    expect(msg).toEqual({ type: "pong" });
  });

  it("round-trips through encode", () => {
    const original = { type: "mask", frameId: "0003", coverage: 0.25 };
    const msg = decodeSidecarMessage(
      encodeSidecarMessage(original as unknown as Record<string, unknown>),
    );
    expect(msg).toEqual(original);
  });

  it("rejects empty lines", () => {
    expect(() => decodeSidecarMessage("")).toThrow();
    expect(() => decodeSidecarMessage("   \n")).toThrow();
  });

  it("rejects invalid JSON", () => {
    expect(() => decodeSidecarMessage("{not json}\n")).toThrow();
  });

  it("rejects non-object payloads", () => {
    expect(() => decodeSidecarMessage('[1,2,3]\n')).toThrow();
  });

  it("rejects messages without a string type", () => {
    expect(() => decodeSidecarMessage('{"id":1}\n')).toThrow();
    expect(() => decodeSidecarMessage('{"type":42}\n')).toThrow();
  });

  it("rejects unknown message types", () => {
    expect(() => decodeSidecarMessage('{"type":"warp_core"}\n')).toThrow(/unknown.*type/i);
  });

  it("accepts every protocol vocabulary type", () => {
    const known = [
      "hello", "ping", "init_session", "add_prompt", "propagate", "shutdown",
      "session_ready", "prompt_accepted", "mask", "propagate_done", "bye", "error",
    ];
    for (const type of known) {
      expect(() => decodeSidecarMessage(`{"type":"${type}"}\n`)).not.toThrow();
    }
  });
});

describe("SidecarRequestError", () => {
  it("carries the sidecar error code and message", () => {
    const err = new SidecarRequestError("unknown_frame", "unknown frameId: 9999");
    expect(err.code).toBe("unknown_frame");
    expect(err.message).toContain("9999");
    expect(err.name).toBe("SidecarRequestError");
  });
});
