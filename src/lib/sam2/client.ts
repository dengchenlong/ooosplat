/**
 * High-level SAM2 sidecar client over a {@link SidecarTransport}.
 *
 * The protocol is strictly request/response (propagate streams one `mask`
 * response per frame followed by `propagate_done`), so the client enforces a
 * single in-flight request at a time and reads responses sequentially.
 */
import {
  encodeSidecarMessage,
  decodeSidecarMessage,
  SidecarRequestError,
  type AddPromptRequest,
  type HelloResponse,
  type MaskResponse,
  type PromptAcceptedResponse,
  type PropagateDoneResponse,
  type PropagateRequest,
  type SidecarFrame,
  type SessionReadyResponse,
} from "./protocol";
import type { SidecarTransport } from "./transport";

export interface PropagateOptions {
  /** Called for every streamed `mask` response. */
  onMask?: (mask: MaskResponse) => void;
  /** Abort: stops consuming responses and disposes the transport. */
  signal?: AbortSignal;
}

export class SidecarClient {
  private transport: SidecarTransport;
  private pending = false;

  constructor(transport: SidecarTransport) {
    this.transport = transport;
  }

  private async request<T extends { type: string }>(
    message: Record<string, unknown> & { type: string },
  ): Promise<T> {
    if (this.pending) {
      throw new Error("a sidecar request is already in flight");
    }
    this.pending = true;
    try {
      this.transport.writeLine(encodeSidecarMessage(message as never));
      const line = await this.transport.readLine();
      if (line === null) {
        throw new Error("sidecar closed its output before responding");
      }
      const response = decodeSidecarMessage(line);
      if (response.type === "error") {
        throw new SidecarRequestError(response.code, response.message, response.inReplyTo);
      }
      return response as T;
    } finally {
      this.pending = false;
    }
  }

  async hello(): Promise<HelloResponse> {
    return this.request<HelloResponse>({ type: "hello", protocolVersion: 1 });
  }

  async ping(): Promise<void> {
    await this.request({ type: "ping" });
  }

  async initSession(sessionId: string, frames: SidecarFrame[]): Promise<SessionReadyResponse> {
    return this.request<SessionReadyResponse>({ type: "init_session", sessionId, frames });
  }

  /**
   * Add a prompt (or re-prompt: a repeated add_prompt on a different frame
   * becomes a new anchor against loop drift; the response's `anchorCount`
   * reports how many anchors the object now carries).
   */
  async addPrompt(request: Omit<AddPromptRequest, "type">): Promise<PromptAcceptedResponse> {
    return this.request<PromptAcceptedResponse>({ ...request, type: "add_prompt" });
  }

  /**
   * Run full-frame propagation, streaming per-frame masks to `onMask`.
   * Resolves with the final `propagate_done` (completed/skipped counts; the
   * sidecar resumes from its checkpoint, so `skipped` counts masks that were
   * already on disk from an interrupted earlier run).
   */
  async propagate(
    request: Omit<PropagateRequest, "type">,
    options: PropagateOptions = {},
  ): Promise<PropagateDoneResponse> {
    if (this.pending) {
      throw new Error("a sidecar request is already in flight");
    }
    const { onMask, signal } = options;
    this.pending = true;
    try {
      this.transport.writeLine(
        encodeSidecarMessage({ ...request, type: "propagate" } as never),
      );
      for (;;) {
        if (signal?.aborted) {
          await this.transport.dispose();
          throw abortError();
        }
        const line = await this.transport.readLine();
        if (line === null) {
          throw new Error("sidecar closed its output during propagation");
        }
        const response = decodeSidecarMessage(line);
        switch (response.type) {
          case "mask":
            onMask?.(response);
            break;
          case "propagate_done":
            return response;
          case "error":
            throw new SidecarRequestError(response.code, response.message, response.inReplyTo);
          default:
            throw new Error(`unexpected response during propagation: ${response.type}`);
        }
      }
    } finally {
      this.pending = false;
    }
  }

  async shutdown(): Promise<void> {
    await this.request({ type: "shutdown" });
  }

  /** Hard-kill the sidecar process (fallback when shutdown is impossible). */
  async dispose(): Promise<void> {
    await this.transport.dispose();
  }
}

function abortError(): Error {
  const error = new Error("propagation aborted");
  error.name = "AbortError";
  return error;
}
