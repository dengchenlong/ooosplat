import { expect, it } from 'vitest';
import { changesOperation, overallPercent, stagePercent, staleOperation } from './progressState';
import type { PipelineEvent } from '../types/pipeline';
const event = (id: number, kind: PipelineEvent['kind'] = 'stage') => ({ kind, stage: 'reconstructing', substage: 'panoramaReconstruction', operation: { id, attempt: id }, stageProgress: 100, indeterminate: false }) as PipelineEvent;
it('only lifecycle events switch operations; late process events cannot restore old state', () => {
  expect(changesOperation(event(1), event(2))).toBe(true);
  expect(changesOperation(event(1), { ...event(1, 'log'), substage: null })).toBe(false);
  expect(staleOperation(event(2), event(1, 'runtime'))).toBe(true);
  expect(staleOperation(event(1), event(2, 'runtime'))).toBe(true);
  expect(staleOperation(event(1), event(2))).toBe(false);
});
it('keeps unknown stage progress unknown and prevents rounding active tasks to 100%', () => {
  expect(stagePercent({ ...event(1), indeterminate: true })).toBeNull();
  expect(stagePercent({ ...event(1), stageProgress: null })).toBeNull();
  expect(overallPercent(99.99, false)?.toFixed(1)).toBe('99.9');
  expect(overallPercent(100, true)).toBe(100);
});
