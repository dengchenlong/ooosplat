import type { SidecarTransport } from "./transport";

/**
 * In-memory transport for client tests: queued responses, recorded writes.
 * (Test double; lives outside `*.test.ts` because `tsc -b` covers all of src/.
 */
export class ScriptedTransport implements SidecarTransport {
  written: string[] = [];
  disposed = false;
  private pending: ((line: string | null) => void)[] = [];
  private queue: string[] = [];

  writeLine(line: string): void {
    this.written.push(line);
  }

  /** Queue one response line (or null for EOF). */
  push(line: string | null): void {
    const waiter = this.pending.shift();
    if (waiter) waiter(line);
    else this.queue.push(line as string);
  }

  readLine(): Promise<string | null> {
    const queued = this.queue.shift();
    if (queued !== undefined) return Promise.resolve(queued);
    return new Promise((resolve) => this.pending.push(resolve));
  }

  async dispose(): Promise<void> {
    this.disposed = true;
    for (const waiter of this.pending.splice(0)) waiter(null);
  }

  requestOf(index: number): Record<string, unknown> {
    return JSON.parse(this.written[index]);
  }
}
