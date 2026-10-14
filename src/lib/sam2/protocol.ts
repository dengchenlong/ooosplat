/**
 * TS mirror of the SAM2 sidecar JSON-lines protocol (version 1).
 *
 * Framing: one UTF-8 JSON object per line, `\\n`-terminated. The authoritative
 * vocabulary lives in `engines/sam2/sidecar/sam2_sidecar/protocol.py`; keep the
 * two in sync when the protocol version bumps.
 */

export const PROTOCOL_VERSION = 1;

/** Request types (client -> sidecar). */
export interface HelloRequest {
  type: "hello";
  protocolVersion: number;
}
export interface PingRequest {
  type: "ping";
}
export interface SidecarFrame {
  frameId: string;
  path: string;
}
export interface InitSessionRequest {
  type: "init_session";
  sessionId: string;
  frames: SidecarFrame[];
}
export interface AddPromptRequest {
  type: "add_prompt";
  sessionId: string;
  objectId: string;
  frameId: string;
  points?: [number, number][];
  pointLabels?: number[];
  box?: [number, number, number, number];
}
export interface PropagateRequest {
  type: "propagate";
  sessionId: string;
  objectId: string;
  masksDir: string;
}
export interface ShutdownRequest {
  type: "shutdown";
}
export type SidecarRequest =
  | HelloRequest
  | PingRequest
  | InitSessionRequest
  | AddPromptRequest
  | PropagateRequest
  | ShutdownRequest;

/** Response types (sidecar -> client). */
export interface HelloResponse {
  type: "hello";
  protocolVersion: number;
  backend: string;
  model: string;
  localOnly: boolean;
  checkpoint?: string;
}
export interface PongResponse {
  type: "pong";
}
export interface SessionReadyResponse {
  type: "session_ready";
  sessionId: string;
  frameCount: number;
}
export interface PromptAcceptedResponse {
  type: "prompt_accepted";
  sessionId: string;
  objectId: string;
  frameId: string;
  anchorCount: number;
}
export interface MaskResponse {
  type: "mask";
  sessionId: string;
  objectId: string;
  frameId: string;
  maskPath: string;
  width: number;
  height: number;
  coverage: number;
}
export interface PropagateDoneResponse {
  type: "propagate_done";
  sessionId: string;
  objectId: string;
  completed: number;
  skipped: number;
}
export interface ByeResponse {
  type: "bye";
}
export interface ErrorResponse {
  type: "error";
  code: string;
  message: string;
  inReplyTo?: string;
}
export type SidecarResponse =
  | HelloResponse
  | PongResponse
  | SessionReadyResponse
  | PromptAcceptedResponse
  | MaskResponse
  | PropagateDoneResponse
  | ByeResponse
  | ErrorResponse;

const KNOWN_TYPES = new Set([
  "hello",
  "ping",
  "pong",
  "init_session",
  "add_prompt",
  "propagate",
  "shutdown",
  "session_ready",
  "prompt_accepted",
  "mask",
  "propagate_done",
  "bye",
  "error",
]);

/** Serialize one request to a newline-terminated JSON line. */
export function encodeSidecarMessage(message: Record<string, unknown>): string {
  return JSON.stringify(message) + "\n";
}

/** Parse one response line. Throws on framing/type violations. */
export function decodeSidecarMessage(line: string): SidecarResponse {
  const stripped = line.replace(/\r$/, "").trim();
  if (!stripped) throw new Error("empty sidecar protocol line");
  let parsed: unknown;
  try {
    parsed = JSON.parse(stripped);
  } catch (error) {
    throw new Error(`invalid sidecar JSON: ${(error as Error).message}`);
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new Error("sidecar protocol message must be a JSON object");
  }
  const type = (parsed as Record<string, unknown>).type;
  if (typeof type !== "string" || !type) {
    throw new Error("sidecar protocol message requires a non-empty string 'type'");
  }
  if (!KNOWN_TYPES.has(type)) {
    throw new Error(`unknown sidecar protocol message type: ${type}`);
  }
  return parsed as SidecarResponse;
}

/** An `error` response turned into an exception. */
export class SidecarRequestError extends Error {
  readonly code: string;
  readonly inReplyTo?: string;

  constructor(code: string, message: string, inReplyTo?: string) {
    super(`sidecar error [${code}]: ${message}`);
    this.name = "SidecarRequestError";
    this.code = code;
    this.inReplyTo = inReplyTo;
  }
}
