// The display model of a suite report (L1) and of a suite comparison: the task × repeat grid,
// each status and verdict as glyph + word (colour only for abnormal), and the figures formatted.
import type { CheckRate, RunRow, RunRowStatus, Spread, SuiteReport, SuiteStatus, Verdict } from './types';

export interface Mark {
  glyph: string;
  word: string;
  /** Abnormal states are the only ones drawn in colour. */
  abnormal: boolean;
}

const RUN: Record<RunRowStatus, Mark> = {
  pending: { glyph: '○', word: 'pending', abnormal: false },
  starting: { glyph: '◔', word: 'starting', abnormal: false },
  running: { glyph: '◐', word: 'running', abnormal: false },
  passed: { glyph: '✓', word: 'passed', abnormal: false },
  failed: { glyph: '✕', word: 'failed', abnormal: true },
  infra_error: { glyph: '⚠', word: 'infra error', abnormal: true },
  interrupted: { glyph: '■', word: 'interrupted', abnormal: true },
  error: { glyph: '⚠', word: 'could not start', abnormal: true },
  missing: { glyph: '?', word: 'run deleted', abnormal: false },
};

export function runMark(status: RunRowStatus): Mark {
  return RUN[status] ?? { glyph: '?', word: status, abnormal: false };
}

const SUITE: Record<SuiteStatus, Mark> = {
  running: { glyph: '◐', word: 'running', abnormal: false },
  finished: { glyph: '■', word: 'finished', abnormal: false },
  stopped: { glyph: '⚠', word: 'stopped', abnormal: true },
};

export const suiteMark = (s: SuiteStatus): Mark => SUITE[s];

const VERDICT: Record<Verdict, Mark> = {
  improved: { glyph: '▲', word: 'improved', abnormal: false },
  regressed: { glyph: '▼', word: 'regressed', abnormal: true },
  same: { glyph: '=', word: 'same', abnormal: false },
  incomparable: { glyph: '–', word: 'n/a', abnormal: false },
};

export const verdictMark = (v: Verdict): Mark => VERDICT[v];

/** A run cell of the grid: its mark, plus a safety violation as its own abnormal mark. */
export interface Cell {
  repeat: number;
  mark: Mark;
  /** "2/3 checks", empty until scored. */
  checks: string;
  unsafe: boolean;
  runId: string | null;
  branchId: string | null;
  title: string;
}

export interface GridRow {
  task: string;
  cells: Cell[];
  /** Scored runs of the task that passed every check. */
  passed: number;
  scored: number;
  violations: number;
}

function cell(r: RunRow): Cell {
  const o = r.outcome;
  const scored = !!o?.scored;
  return {
    repeat: r.repeat,
    mark: runMark(r.status),
    checks: scored ? `${o!.checks_passed}/${o!.checks_total}` : '',
    unsafe: scored && o!.safety_violations > 0,
    runId: r.run_id,
    branchId: r.branch_id,
    title: r.error ?? o?.error ?? '',
  };
}

/** One row per task, in the suite's order, each with its repeats in order. */
export function grid(report: SuiteReport): GridRow[] {
  return report.tasks.map((task) => {
    const runs = report.runs.filter((r) => r.task_id === task).sort((a, b) => a.repeat - b.repeat);
    const scored = runs.filter((r) => r.outcome?.scored);
    return {
      task,
      cells: runs.map(cell),
      passed: scored.filter((r) => r.outcome!.status === 'passed').length,
      scored: scored.length,
      violations: scored.reduce((n, r) => n + r.outcome!.safety_violations, 0),
    };
  });
}

/** Goal checks first, then safety checks, each in the order the report lists them. */
export function checkRows(checks: CheckRate[], task: string): CheckRate[] {
  const of = checks.filter((c) => c.task_id === task);
  return [...of.filter((c) => !c.safety), ...of.filter((c) => c.safety)];
}

export function pct(x: number | null | undefined): string {
  return x == null ? '–' : `${Math.round(x * 100)}%`;
}

/** "+25 pts", "−10 pts", "±0": a change of a rate in percentage points. */
export function pts(change: number | null): string {
  if (change == null) return '–';
  const p = Math.round(change * 100);
  return p === 0 ? '±0' : `${p > 0 ? '+' : '−'}${Math.abs(p)} pts`;
}

export type Unit = 's' | 'tok' | 'usd' | 'n';

export function fmt(x: number | null, unit: Unit): string {
  if (x == null) return '–';
  if (unit === 's') return `${x.toFixed(1)} s`;
  if (unit === 'usd') return `$${x.toFixed(x < 0.1 ? 4 : 2)}`;
  if (unit === 'tok') return x >= 10_000 ? `${(x / 1000).toFixed(1)}k` : `${Math.round(x)}`;
  return Number.isInteger(x) ? `${x}` : x.toFixed(2);
}

/** "3.0 s (2.0–4.0)"; "2 unknown" added when some runs lack the figure. */
export function spreadLine(s: Spread, unit: Unit): string {
  if (s.mean == null) return s.unknown ? `unknown (${s.unknown})` : '–';
  const range = s.min === s.max ? '' : ` (${fmt(s.min, unit)}–${fmt(s.max, unit)})`;
  const unknown = s.unknown ? `, ${s.unknown} unknown` : '';
  return `${fmt(s.mean, unit)}${range}${unknown}`;
}

/** A signed change of a figure: "+1.2 s", "−300"; "±0" when it rounds to nothing. */
export function change(x: number | null, unit: Unit): string {
  if (x == null) return '–';
  const size = fmt(Math.abs(x), unit);
  // A change too small to show at the figure's precision is no change.
  if (size === fmt(0, unit)) return '±0';
  return `${x > 0 ? '+' : '−'}${size}`;
}
