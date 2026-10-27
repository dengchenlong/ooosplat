import type { TaskDraft } from './taskWorkspace';
import type { ImageMatchingStrategy, VideoProjection } from '../types/pipeline';

export type PreflightKind = 'smallImages' | 'imageOrder' | 'reshootCondition' | 'reshootCamera';
export interface PreflightQuestion { id: string; kind: PreflightKind; draft: TaskDraft; snapshot: string }
export type PreflightAnswers = Record<string, string>;
export const projectionIdentity = (draft: TaskDraft) => draft.projectionDetection?.sourceIdentity || draft.inputPath || '';
export const preflightSnapshot = (draft: TaskDraft) => JSON.stringify([draft.id, draft.inputPath, draft.inputType, draft.quality, draft.plannerEnabled, projectionIdentity(draft), draft.imageSequence?.imageCount, draft.projectionDetection?.confirmedErp, draft.projectionDetection?.supportedErp, resolvedProjection(draft), draft.reshootSource?.sourceIdentity, draft.reshootPlan?.sourceIdentity, draft.reshootPlan?.imageCount, draft.reshootSource?.warmAvailable]);
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
  return drafts.filter(d => !d.running && !!d.inputPath).flatMap(draft => draft.kind === 'reshoot' ? [
    { id: `${draft.id}:reshootCondition`, kind: 'reshootCondition' as const, draft, snapshot: preflightSnapshot(draft) },
    ...(draft.reshootSource?.projection === 'equirectangular' ? [] : [{ id: `${draft.id}:reshootCamera`, kind: 'reshootCamera' as const, draft, snapshot: preflightSnapshot(draft) }]),
    ...((draft.inputType === 'images' && (draft.reshootPlan?.imageCount ?? draft.imageSequence?.imageCount ?? 0) > 100) ? [{ id: `${draft.id}:imageOrder`, kind: 'imageOrder' as const, draft, snapshot: preflightSnapshot(draft) }] : []),
  ] : checks
    .filter(check => check.applies(draft))
    .map(check => ({ id: `${draft.id}:${check.kind}`, kind: check.kind, draft, snapshot: preflightSnapshot(draft) })));
}
export function answerIsValid(question: PreflightQuestion, answer?: string) {
  if (question.kind === 'reshootCondition') return ['good','appearance','geometry','unsure'].includes(answer ?? '');
  if (question.kind === 'reshootCamera') return ['same','different','unsure'].includes(answer ?? '');
  return question.kind === 'imageOrder' ? answer === 'sequential' || answer === 'exhaustive' : answer === 'accepted';
}
export function imageMatchingStrategy(draft: TaskDraft, answers: PreflightAnswers): ImageMatchingStrategy {
  return draft.inputType === 'images' && (draft.reshootPlan?.imageCount ?? draft.imageSequence?.imageCount ?? 0) > 100 && answers[`${draft.id}:imageOrder`] === 'sequential' ? 'sequential' : 'exhaustive';
}
