/** One preview instance owns its tickets. No timers can steal a later focus. */
export class PreviewEditFocus {
  private revision = 0;
  private inputRevision = 0;
  private disposed = false;
  private pending: { revision: number; inputRevision: number; origin: Element | null } | null = null;

  constructor(private readonly focusCanvas: () => boolean, private readonly allowed: () => boolean) {
    document.addEventListener("focusin", this.interrupted, true);
    document.addEventListener("pointerdown", this.interrupted, true);
    window.addEventListener("blur", this.interrupted);
    document.addEventListener("visibilitychange", this.interrupted);
  }

  private interrupted = () => { this.inputRevision++; this.pending = null; };

  begin(): () => void {
    const ticket = { revision: ++this.revision, inputRevision: this.inputRevision, origin: document.activeElement };
    this.pending = null;
    return () => {
      if (this.disposed || ticket.revision !== this.revision || ticket.inputRevision !== this.inputRevision) return;
      this.pending = ticket;
      this.flush();
    };
  }

  flush(): void {
    const ticket = this.pending;
    if (!ticket || this.disposed || document.hidden || !this.allowed()) return;
    if (ticket.revision !== this.revision || ticket.inputRevision !== this.inputRevision
      || (document.activeElement !== ticket.origin && document.activeElement !== document.body)) {
      this.pending = null;
      return;
    }
    if (this.focusCanvas()) this.pending = null;
  }

  destroy(): void {
    this.disposed = true;
    this.pending = null;
    document.removeEventListener("focusin", this.interrupted, true);
    document.removeEventListener("pointerdown", this.interrupted, true);
    window.removeEventListener("blur", this.interrupted);
    document.removeEventListener("visibilitychange", this.interrupted);
  }
}
