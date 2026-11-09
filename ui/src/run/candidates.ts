// An optimisation run's candidates, from its steps: every tool result that carries an `objective`
// (`{label, value, ci95?, unit?, better}`, docs/protocol.md "Optimisation studies") is one candidate
// evaluated, in step order, with the best feasible value so far. The check's measure with the same
// label gives the target and the recommendation's verified value. Pure functions only.
import type { CheckPayload, Event, Measure, ToolCallPayload } from '../types';

export interface Objective {
  label: string;
  unit: string;
  better: 'higher' | 'lower';
}

export interface Candidate {
  seq: number;
  /** The simulation's number, as the tool counted it (else its order among the candidates). */
  n: number;
  value: number;
  ci: number | null;
  /** False when the candidate missed a limit judged after it ran (`feasible: false`). */
  feasible: boolean;
  /** What it changed, "line2.batch_size 240 · line2.pm_hours 2"; empty for the baseline. */
  changes: string;
  /** The best feasible value up to and including this one; null before any feasible one. */
  best: number | null;
}

export interface Study {
  objective: Objective;
  candidates: Candidate[];
  /** From the run's check, once scored. */
  target: number | null;
  verified: number | null;
}

type Raw = { objective?: unknown; simulation?: unknown; feasible?: unknown; changes?: unknown };

function objectiveOf(r: unknown): { label: string; value: number; ci95?: number | null; unit?: string; better?: string } | null {
  const o = (r as Raw | null)?.objective as Record<string, unknown> | undefined;
  return o && typeof o === 'object' && typeof o.value === 'number' && typeof o.label === 'string' ? (o as never) : null;
}

const changesLine = (c: unknown) => (c && typeof c === 'object' ? Object.entries(c).map(([k, v]) => `${k} ${Array.isArray(v) ? v.join('/') : String(v)}`).join(' · ') : '');

/** The check's measure that reports this objective (same label), if the run was scored. */
function measureOf(events: Event[], label: string): Measure | null {
  const check = [...events].reverse().find((e) => e.kind === 'check')?.payload as CheckPayload | undefined;
  return check?.checks?.find((c) => c.measure?.label === label)?.measure ?? null;
}

/** The study a run made, or null when no step evaluated a candidate. */
export function studyOf(events: Event[]): Study | null {
  const calls = events.filter((e) => e.kind === 'tool_call' && !(e.payload as ToolCallPayload).error);
  const found = calls.flatMap((e) => {
    const p = e.payload as ToolCallPayload;
    const o = objectiveOf(p.result);
    return o ? [{ e, p, o }] : [];
  });
  if (!found.length) return null;
  const first = found[0].o;
  const objective: Objective = { label: first.label, unit: first.unit ?? '', better: first.better === 'lower' ? 'lower' : 'higher' };
  const better = (a: number, b: number) => (objective.better === 'higher' ? a > b : a < b);
  let best: number | null = null;
  const candidates = found.map(({ e, p, o }, i) => {
    const r = p.result as Raw;
    const feasible = r.feasible !== false;
    if (feasible && (best == null || better(o.value, best))) best = o.value;
    return { seq: e.seq, n: typeof r.simulation === 'number' ? r.simulation : i + 1, value: o.value, ci: o.ci95 ?? null, feasible, changes: changesLine(r.changes), best };
  });
  const m = measureOf(events, objective.label);
  return { objective, candidates, target: m?.target ?? null, verified: m ? m.value : null };
}

/** "86.4%" or "3.2 h". */
export function objectiveText(v: number | null, unit: string): string {
  if (v == null) return '–';
  const n = Number.isInteger(v) ? String(v) : v.toFixed(1);
  return unit === '%' ? `${n}%` : `${n}${unit ? ` ${unit}` : ''}`;
}

/** Round ticks across [lo, hi]: about `count` of them at 1, 2 or 5 × 10^k. */
export function ticks(lo: number, hi: number, count = 4): number[] {
  if (!(hi > lo)) return [lo];
  const raw = (hi - lo) / count;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 5, 10].map((m) => m * mag).find((s) => s >= raw) ?? raw;
  const out: number[] = [];
  for (let v = Math.ceil(lo / step) * step; v <= hi + 1e-9; v += step) out.push(Math.round(v / step) * step);
  return out;
}
