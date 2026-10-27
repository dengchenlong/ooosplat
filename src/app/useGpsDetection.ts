import { useEffect } from 'react';
import { detectImageGps } from '../lib/backend';
import type { TaskDraft } from './taskWorkspace';

// GPS is an independent, optional hint. Neither selection nor generation waits
// for it. Share requests across StrictMode mounts, quality replans and drafts.
const requests = new Map<string, Promise<boolean>>();
export function useGpsDetection(drafts: TaskDraft[], update: (id: string, patch: Partial<TaskDraft>) => void) {
  useEffect(() => {
    let active = true;
    for (const draft of drafts) {
      const identity = draft.projectionDetection?.sourceIdentity;
      if (draft.inputType !== 'images' || !draft.inputPath || !identity || draft.needsValidation
        || draft.gpsDetection?.sourceIdentity === identity) continue;
      const key = JSON.stringify([draft.inputPath, identity]);
      let request = requests.get(key);
      if (!request) {
        if (requests.size >= 32) requests.delete(requests.keys().next().value!);
        request = Promise.resolve().then(() => detectImageGps(draft.inputPath!, identity)).catch(() => false);
        requests.set(key, request);
      }
      void request.then(detected => {
        if (active) update(draft.id, { gpsDetection: { sourceIdentity: identity, detected } });
      });
    }
    return () => { active = false; };
  }, [drafts, update]);
}
