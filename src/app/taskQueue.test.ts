import { describe, expect, it } from "vitest";
import { createGenerationDraft, createReshootDraft, type TaskDraft } from "./taskWorkspace";
import { displayStatusForDraft, draftIsRunnable, firstQueueInputIssue, moveDraft, queuedDraftIds, reorderDrafts } from "./taskQueue";

const readyGeneration = (ordinal: number): TaskDraft => ({
  ...createGenerationDraft(ordinal), id: `g${ordinal}`, inputPath: `video-${ordinal}.mp4`, plan: {} as TaskDraft["plan"],
});

const readyReshoot = (ordinal: number): TaskDraft => ({
  ...createReshootDraft(ordinal, "source", "source", "balanced"), id: `r${ordinal}`, inputPath: `images-${ordinal}`,
  needsValidation: false,
  reshootSource: { eligible: true } as TaskDraft["reshootSource"],
  reshootPlan: { compatible: true } as TaskDraft["reshootPlan"],
});

describe("task queue ordering", () => {
  it('does not skip a reshoot that is checking, incompatible or has an unanswered projection', () => {
    const draft = readyReshoot(1);
    expect(firstQueueInputIssue([{ ...draft, inputChecking: true }, readyGeneration(2)])?.reason).toBe('checking');
    expect(firstQueueInputIssue([{ ...draft, reshootPlan: null }, readyGeneration(2)])?.reason).toBe('analysis');
    const ambiguous:TaskDraft = { ...draft, projectionDetection:{sourceIdentity:'new',requiresChoice:true,supportedErp:true,confirmedErp:false,reason:'',previewJpeg:[]},reshootSource:{...draft.reshootSource!,projection:'equirectangular'} };
    expect(firstQueueInputIssue([ambiguous,readyGeneration(2)])?.reason).toBe('projection');
    expect(draftIsRunnable({...ambiguous,projectionAnswer:{identity:'new',projection:'perspective'}})).toBe(false);
    expect(draftIsRunnable({...ambiguous,projectionAnswer:{identity:'new',projection:'equirectangular'}})).toBe(true);
  });
  it('blocks panorama image detection and unanswered types without skipping ahead',()=>{
    const images:TaskDraft={...readyGeneration(1),inputType:'images',inputPath:'images',projectionDetection:{sourceIdentity:'photos-v1',confirmedErp:false,requiresChoice:true,supportedErp:true,reason:'',previewJpeg:[]}};
    expect(draftIsRunnable(images)).toBe(false);
    expect(firstQueueInputIssue([images,readyGeneration(2)])?.reason).toBe('projection');
    const answered:TaskDraft={...images,projectionAnswer:{identity:'photos-v1',projection:'equirectangular'}};
    expect(draftIsRunnable(answered)).toBe(true);
    expect(firstQueueInputIssue([{...answered,inputChecking:true},readyGeneration(2)])?.reason).toBe('checking');
    expect(draftIsRunnable({...answered,projectionDetection:{...images.projectionDetection!,sourceIdentity:'photos-v2'}})).toBe(false);
    expect(draftIsRunnable({...answered,projectionDetection:{...images.projectionDetection!,supportedErp:false,unsupported:true}})).toBe(false);
  });
  it("reorders by pointer insertion boundary without changing task identity", () => {
    const drafts = [readyGeneration(1), readyGeneration(2), readyGeneration(3)];
    expect(reorderDrafts(drafts, "g1", 3).map((draft) => draft.id)).toEqual(["g2", "g3", "g1"]);
    expect(reorderDrafts(drafts, "g3", 0).map((draft) => draft.id)).toEqual(["g3", "g1", "g2"]);
    expect(reorderDrafts(drafts, "g2", 2)).toBe(drafts);
  });

  it("does not move the running task, but lets another task cross it", () => {
    const drafts = [readyGeneration(1), { ...readyGeneration(2), running: true }, readyGeneration(3)];
    expect(reorderDrafts(drafts, "g2", 0)).toBe(drafts);
    expect(reorderDrafts(drafts, "g3", 1).map((draft) => draft.id)).toEqual(["g1", "g3", "g2"]);
  });

  it("supports accessible one-step keyboard moves", () => {
    const drafts = [readyGeneration(1), readyGeneration(2), readyGeneration(3)];
    expect(moveDraft(drafts, "g2", -1).map((draft) => draft.id)).toEqual(["g2", "g1", "g3"]);
    expect(moveDraft(drafts, "g2", 1).map((draft) => draft.id)).toEqual(["g1", "g3", "g2"]);
  });
});

describe("task queue status", () => {
  it('checks every populated standby draft in order while ignoring empty and running drafts', () => {
    const running = { ...readyGeneration(1), running: true, inputChecking: true };
    const unresolved = { ...readyGeneration(3), projectionDetection: { requiresChoice: true, supportedErp: true, confirmedErp: false, reason: '', previewJpeg: [] } };
    expect(firstQueueInputIssue([createGenerationDraft(1), running, { ...readyGeneration(2), inputChecking: true }, unresolved])?.reason).toBe('checking');
    expect(firstQueueInputIssue([running, readyGeneration(2), unresolved])?.draft.id).toBe(unresolved.id);
    expect(firstQueueInputIssue([{ ...unresolved, projectionAnswer: { identity: unresolved.inputPath!, projection: 'perspective' } }])).toBeNull();
    expect(firstQueueInputIssue([{ ...readyGeneration(2), inputError: 'unreadable' }])?.reason).toBe('analysis');
  });
  it("stops at an ambiguous panorama until the user chooses a projection", () => {
    const draft = { ...readyGeneration(1), projectionDetection: { requiresChoice: true, supportedErp: true, confirmedErp: false, reason: "2:1", previewJpeg: [] } };
    expect(draftIsRunnable(draft)).toBe(false);
    expect(draftIsRunnable({ ...draft, projectionAnswer: { identity: draft.inputPath!, projection: 'perspective' } })).toBe(true);
    expect(draftIsRunnable({ ...draft, projectionAnswer: { identity: draft.inputPath!, projection: 'equirectangular' } })).toBe(true);
    expect(draftIsRunnable({ ...draft, projectionAnswer: { identity: draft.inputPath!, projection: 'equirectangular' }, projectionDetection: { ...draft.projectionDetection, supportedErp: false } })).toBe(false);
  });
  it("queues a contiguous ready generation/reshoot prefix and stops at the first barrier", () => {
    const active = { ...readyGeneration(1), running: true };
    const blocked = { ...readyGeneration(3), inputPath: null, plan: null };
    const drafts = [active, readyReshoot(2), blocked, readyGeneration(4)];
    expect([...queuedDraftIds(drafts, active.id, true)]).toEqual(["r2"]);
    expect(displayStatusForDraft(active, new Set())).toBe("running");
    expect(displayStatusForDraft({ ...blocked, inputChecking: true }, new Set())).toBe("preparing");
  });

  it("does not queue while disabled, without an active draft, or without prerequisites", () => {
    const drafts = [{ ...readyGeneration(1), running: true }, readyGeneration(2)];
    expect(queuedDraftIds(drafts, "g1", false).size).toBe(0);
    expect(queuedDraftIds(drafts, null, true).size).toBe(0);
    expect(queuedDraftIds(drafts, "g1", true, false).size).toBe(0);
  });

  it("requires completed input checks and compatible input", () => {
    expect(draftIsRunnable({ ...readyGeneration(1), inputChecking: true })).toBe(false);
    expect(draftIsRunnable({ ...readyReshoot(2), reshootPlan: null })).toBe(false);
    expect(draftIsRunnable(readyReshoot(3))).toBe(true);
  });
});
