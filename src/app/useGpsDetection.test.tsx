// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useGpsDetection } from './useGpsDetection';
import { createGenerationDraft, type TaskDraft } from './taskWorkspace';

const detect = vi.hoisted(() => vi.fn());
vi.mock('../lib/backend', () => ({ detectImageGps: detect }));
let root: Root; let container: HTMLDivElement;
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
beforeEach(() => { detect.mockReset(); container = document.createElement('div'); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
function Probe({ drafts, update }: { drafts: TaskDraft[]; update: (id: string, patch: Partial<TaskDraft>) => void }) { useGpsDetection(drafts, update); return null; }
function draft(identity: string): TaskDraft { return { ...createGenerationDraft(1), inputType: 'images', inputPath: `E:/gps/${identity}`, projectionDetection: { sourceIdentity: identity, confirmedErp: false, supportedErp: true, unsupported: false, requiresChoice: false, reason: '', previewJpeg: [] } }; }

it('deduplicates quality replans, is optional, and does not inspect videos', async () => {
  const update = vi.fn(); const d = draft('quality'); detect.mockResolvedValue(true);
  await act(async () => root.render(<Probe drafts={[d]} update={update} />));
  await act(async () => root.render(<Probe drafts={[{ ...d, quality: 'high', plannerEnabled: false }]} update={update} />));
  expect(detect).toHaveBeenCalledTimes(1);
  expect(update).toHaveBeenLastCalledWith(d.id, { gpsDetection: { sourceIdentity: 'quality', detected: true } });
  await act(async () => root.render(<Probe drafts={[{ ...draft('video'), inputType: 'video' }]} update={update} />));
  expect(detect).toHaveBeenCalledTimes(1);
});

it('ignores late results after replacing media or unmounting', async () => {
  const update = vi.fn(); let done!: (v: boolean) => void;
  detect.mockImplementationOnce(() => new Promise<boolean>(resolve => { done = resolve; })).mockResolvedValueOnce(false);
  await act(async () => root.render(<Probe drafts={[draft('old')]} update={update} />));
  const next = draft('new'); await act(async () => root.render(<Probe drafts={[next]} update={update} />));
  await act(async () => done(true));
  expect(update.mock.calls).toEqual([[next.id, { gpsDetection: { sourceIdentity: 'new', detected: false } }]]);
  let late!: (v: boolean) => void; detect.mockImplementationOnce(() => new Promise<boolean>(resolve => { late = resolve; }));
  await act(async () => root.render(<Probe drafts={[draft('unmount')]} update={update} />));
  await act(async () => root.render(null)); await act(async () => late(true));
  expect(update).toHaveBeenCalledTimes(1);
});

it('treats inspection errors as no optional hint, not a generation error', async () => {
  detect.mockRejectedValue(new Error('unreadable metadata')); const update = vi.fn(); const d = draft('broken');
  await act(async () => root.render(<Probe drafts={[d]} update={update} />));
  expect(update).toHaveBeenCalledWith(d.id, { gpsDetection: { sourceIdentity: 'broken', detected: false } });
});
