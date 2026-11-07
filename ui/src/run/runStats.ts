// The run header's figures: steps, reads and writes, abnormal actions, and the check.
import { actionsOf } from '../control/actions';
import type { Event } from '../types';

export interface RunStats {
  steps: number;
  reads: number;
  writes: number;
  /** Refused or failed actions. */
  abnormal: number;
  check: { passed: boolean; ok: number; total: number } | null;
}

export function runStats(events: Event[], startSeq: number | null): RunStats {
  const own = events.filter((e) => e.kind !== 'run_start' && (startSeq == null || e.seq >= startSeq));
  const actions = actionsOf(events, startSeq).filter((a) => !a.inherited);
  const check = [...events].reverse().find((e) => e.kind === 'check');
  const p = (check?.payload ?? {}) as { passed?: boolean; checks?: { passed: boolean }[] };
  const list = Array.isArray(p.checks) ? p.checks : [];
  return {
    steps: own.length,
    reads: actions.filter((a) => a.effect === 'read').length,
    writes: actions.filter((a) => a.effect === 'write').length,
    abnormal: actions.filter((a) => a.outcome !== 'ok').length,
    check: check ? { passed: !!p.passed, ok: list.filter((c) => c.passed).length, total: list.length } : null,
  };
}
