import { beginGaussianEditSave, commitGaussianEditSave, saveGaussianTransform } from "../../lib/backend";
import { getCurrentLocale, translate } from "../../i18n";
import { cloneCrop, useGaussianTransformStore } from "../../stores/gaussianTransformStore";

/** One project-scoped writer for autosave, explicit save and export/exit barriers.
 * Subscribing to committed revisions avoids waiting for React effects to enqueue
 * the final input edit. Retrying never fetches the old on-disk deletion mask.
 */
export class PreviewEditPersistence {
  private queue: Promise<void> = Promise.resolve();
  private queuedRevision = 0;
  private savedEditSerial = 0;
  private pending = 0;
  private disposed = false;
  private unsubscribe: () => void;

  constructor(private readonly projectId: string) {
    this.unsubscribe = useGaussianTransformStore.subscribe((state, previous) => {
      if (state.revision !== previous.revision && state.descriptor?.projectId === projectId) this.enqueue();
    });
  }

  private current() {
    const state = useGaussianTransformStore.getState();
    return !this.disposed && state.descriptor?.projectId === this.projectId ? state : null;
  }

  private enqueue(force = false) {
    const state = this.current();
    if (!state || (!force && state.revision === this.queuedRevision)) return;
    const revision = state.revision;
    this.queuedRevision = revision;
    const transform = { ...state.transform, position: [...state.transform.position] as [number, number, number], rotation: [...state.transform.rotation] as [number, number, number] };
    const serial = state.editChangeSerial;
    const crop = cloneCrop(state.editing.crop);
    const mask = serial !== this.savedEditSerial || force ? state.deletedMask.slice() : null;
    this.pending++;
    state.setSaveState("saving");
    this.queue = this.queue.then(async () => {
      if (!this.current()) return;
      try {
        await saveGaussianTransform(this.projectId, transform);
        const current = this.current();
        if (!current) return;
        if (mask && (serial !== this.savedEditSerial || force)) {
          const reservation = await beginGaussianEditSave(this.projectId, crop, current.editing.revision);
          if (reservation.expectedMaskBytes !== mask.byteLength) throw new Error(translate(getCurrentLocale(), "viewer.maskLength"));
          const editing = await commitGaussianEditSave(reservation.editId, mask);
          this.savedEditSerial = serial;
          this.current()?.applySavedEditing(editing, serial);
        }
        const latest = this.current();
        if (latest) {
          const dirty = latest.revision !== revision || latest.transactionStart !== null || latest.cropTransactionStart !== undefined;
          latest.setSaveState(this.pending > 1 ? "saving" : dirty ? "dirty" : "saved");
        }
      } catch (error) {
        this.current()?.setSaveState("error", error instanceof Error ? error.message : String(error));
      } finally { this.pending--; }
    });
  }

  async flush(retry = false) {
    if (!this.current()) throw new Error(translate(getCurrentLocale(), "viewer.editsUnsaved"));
    this.enqueue(retry);
    let queued: Promise<void>;
    do { queued = this.queue; await queued; } while (queued !== this.queue);
    const state = this.current();
    if (!state || state.saveState === "error") throw new Error(state?.saveError ?? translate(getCurrentLocale(), "viewer.editsUnsaved"));
    if (state.saveState === "dirty" && !state.transactionStart && state.cropTransactionStart === undefined
      && state.revision === this.queuedRevision) state.setSaveState("saved");
  }

  destroy() {
    this.disposed = true;
    this.unsubscribe();
  }
}
