// The run header's figures: steps, reads and writes, abnormal actions, and the check.
import { actionsOf } from '../control/actions';
import type { Mark } from '@/lib/mark';
import type { Event } from '../types';

export interface RunStats {
  steps: number;
  reads: number;
  writes: number;
  /** Refused or failed actions. */
  abnormal: number;
  check: { passed: boolean; ok: number; total: number } | null;
  /** Safety checks the run violated, of those it was scored on. */
  safety: { violated: number; total: number } | null;
}

export function runStats(events: Event[], startSeq: number | null): RunStats {
  const own = events.filter((e) => e.kind !== 'run_start' && e.kind !== 'world_view' && (startSeq == null || e.seq >= startSeq));
  const actions = actionsOf(events, startSeq).filter((a) => !a.inherited);
  const check = [...events].reverse().find((e) => e.kind === 'check');
  const p = (check?.payload ?? {}) as { passed?: boolean; checks?: { passed: boolean; safety?: boolean }[] };
  const list = Array.isArray(p.checks) ? p.checks : [];
  const safety = list.filter((c) => c.safety);
  return {
    steps: own.length,
    reads: actions.filter((a) => a.effect === 'read').length,
    writes: actions.filter((a) => a.effect === 'write').length,
    abnormal: actions.filter((a) => a.outcome !== 'ok').length,
    check: check ? { passed: !!p.passed, ok: list.filter((c) => c.passed).length, total: list.length } : null,
    safety: safety.length ? { violated: safety.filter((c) => !c.passed).length, total: safety.length } : null,
  };
}

/** "✓ passed 13/13", "✕ failed 11/13": the check as a mark. */
export function checkMark(c: NonNullable<RunStats['check']>): Mark {
  const count = c.total ? ` ${c.ok}/${c.total}` : '';
  return c.passed ? { glyph: '✓', word: `Passed${count}`, abnormal: false } : { glyph: '✕', word: `Failed${count}`, abnormal: true };
}

/** "◆ 1 of 4 violated", "0 of 4 violated". */
export function safetyLine(s: NonNullable<RunStats['safety']>): string {
  return `${s.violated ? '◆ ' : ''}${s.violated} of ${s.total} violated`;
}
