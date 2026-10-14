/**
 * Line-oriented transport between the TS host and the SAM2 sidecar process.
 *
 * The client only ever speaks complete lines, so any implementation that can
 * write a line and asynchronously produce the next response line works:
 * the Node child-process transport (`nodeTransport.ts`) for hosts with
 * process access, and scripted in-memory transports in tests.
 */
export interface SidecarTransport {
  /** Write one newline-terminated JSON request line. */
  writeLine(line: string): void;
  /**
   * Resolve with the next response line (without the trailing newline), or
   * `null` when the sidecar closed its stdout.
   */
  readLine(): Promise<string | null>;
  /** Kill the sidecar and release resources. Idempotent. */
  dispose(): Promise<void>;
}
