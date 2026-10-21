import type { TaskDraft } from './taskWorkspace';
import type { ImageMatchingStrategy, VideoProjection } from '../types/pipeline';

export type PreflightKind = 'smallImages' | 'imageOrder';
export interface PreflightQuestion { id: string; kind: PreflightKind; draft: TaskDraft; snapshot: string }
export type PreflightAnswers = Record<string, string>;
export const projectionIdentity = (draft: TaskDraft) => draft.projectionDetection?.sourceIdentity || draft.inputPath || '';
export const preflightSnapshot = (draft: TaskDraft) => JSON.stringify([draft.id, draft.inputPath, draft.inputType, draft.quality, draft.plannerEnabled, projectionIdentity(draft), draft.imageSequence?.imageCount, draft.projectionDetection?.confirmedErp, draft.projectionDetection?.supportedErp, resolvedProjection(draft)]);
export function resolvedProjection(draft: TaskDraft): VideoProjection {
  const detection = draft.projectionDetection;
  if (detection?.confirmedErp) return 'equirectangular';
  const answer = draft.projectionAnswer;
  if (answer?.identity === projectionIdentity(draft) && (answer.projection !== 'equirectangular' || detection?.supportedErp)) return answer.projection;
  return 'auto';
}
// Ordered registry: add another check without coupling rendering to task startup.
const checks: Array<{ kind: PreflightKind; applies: (draft: TaskDraft) => boolean }> = [
  { kind: 'smallImages', applies: d => d.inputType === 'images' && !!d.imageSequence && d.imageSequence.imageCount < 30 },
  { kind: 'imageOrder', applies: d => d.inputType === 'images' && (d.imageSequence?.imageCount ?? 0) > 100 },
];
export function collectPreflightQuestions(drafts: TaskDraft[]): PreflightQuestion[] {
  return drafts.filter(d => d.kind === 'generation' && !d.running && !!d.inputPath).flatMap(draft => checks
    .filter(check => check.applies(draft))
    .map(check => ({ id: `${draft.id}:${check.kind}`, kind: check.kind, draft, snapshot: preflightSnapshot(draft) })));
}
export function answerIsValid(question: PreflightQuestion, answer?: string) {
  return question.kind === 'imageOrder' ? answer === 'sequential' || answer === 'exhaustive' : answer === 'accepted';
}
export function imageMatchingStrategy(draft: TaskDraft, answers: PreflightAnswers): ImageMatchingStrategy {
  return draft.inputType === 'images' && (draft.imageSequence?.imageCount ?? 0) > 100 && answers[`${draft.id}:imageOrder`] === 'sequential' ? 'sequential' : 'exhaustive';
}
