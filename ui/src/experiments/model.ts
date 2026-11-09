// The display model of an experiment: one column per agent setup (its pass rate with a 95% range,
// a cell per run, the figures that matter), the checks side by side, and the setups a new
// experiment is started with. Pure functions only; the pages draw what these return.
import { duration, fmt } from '@/lib/format';
import { runLine } from '../home/summary';
import type { CheckRate, RunRow, SuiteReport } from '../control/types';
import { withRules } from '../team';
import type { Team } from '../types';

export type RunState = 'passed' | 'failed' | 'running' | 'pending' | 'unscored';

/** One run of a setup, as a cell of its strip. */
export interface RunCell {
  key: string;
  state: RunState;
  /** "8/9", empty until scored. */
  checks: string;
  violations: number;
  denials: number;
  flags: number;
  /** The run's page; null before it started. */
  href: string | null;
  title: string;
}

export interface Column {
  id: string;
  name: string;
  /** "6 agents · Claude Code · Sonnet". */
  team: string;
  passed: number;
  scored: number;
  /** Runs asked for. */
  total: number;
  rate: number | null;
  /** The pass rate's 95% range (Wilson), null before a run is scored. */
  range: [number, number] | null;
  runs: RunCell[];
  checksMean: number | null;
  checksTotal: number;
  cost: number | null;
  time: number | null;
  violations: number;
  denials: number;
  flags: number;
  unscored: number;
}

/** The 95% Wilson score interval of `k` passes in `n` runs. */
export function wilson(k: number, n: number, z = 1.96): [number, number] | null {
  if (n <= 0) return null;
  const p = k / n;
  const d = 1 + (z * z) / n;
  const centre = (p + (z * z) / (2 * n)) / d;
  const half = (z * Math.sqrt((p * (1 - p)) / n + (z * z) / (4 * n * n))) / d;
  return [Math.max(0, centre - half), Math.min(1, centre + half)];
}

const LIVE = new Set(['starting', 'running']);

function stateOf(r: RunRow): RunState {
  const o = r.outcome;
  if (o?.scored) return o.status === 'passed' ? 'passed' : 'failed';
  if (r.status === 'pending') return 'pending';
  return LIVE.has(r.status) ? 'running' : 'unscored';
}

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;

const STATE_WORD: Record<RunState, string> = { passed: 'Passed', failed: 'Failed', running: 'Running', pending: 'Not run yet', unscored: 'Not scored' };

/** What a run came to, in counts worth naming: "1/2 checks", "2 denials", "$0.009". */
function outcomeParts(o: RunRow['outcome']): string[] {
  if (!o) return [];
  const counts: [number, string][] = [[o.safety_violations, 'safety violation'], [o.denials, 'denial'], [o.flags, 'flag']];
  return [
    ...(o.scored ? [`${o.checks_passed}/${o.checks_total} checks`] : []),
    ...counts.filter(([n]) => n > 0).map(([n, w]) => plural(n, w)),
    ...(o.cost_usd != null ? [fmt(o.cost_usd, 'usd')] : []),
  ];
}

function cellTitle(i: number, state: RunState, r: RunRow): string {
  const line = [`Run ${i + 1}`, STATE_WORD[state], ...outcomeParts(r.outcome)].join(' · ');
  const why = r.error ?? r.outcome?.error;
  return why ? `${line}\n${why}` : line;
}

function cell(r: RunRow, i: number): RunCell {
  const o = r.outcome;
  const state = stateOf(r);
  return {
    key: `${r.task_id}/${r.variant ?? 0}/${r.repeat}`,
    state,
    checks: o?.scored ? `${o.checks_passed}/${o.checks_total}` : '',
    violations: o?.safety_violations ?? 0,
    denials: o?.denials ?? 0,
    flags: o?.flags ?? 0,
    href: r.run_id && r.branch_id ? `/runs/${r.run_id}/${r.branch_id}` : null,
    title: cellTitle(i, state, r),
  };
}

/** A setup's column, from its suite report. */
export function columnOf(r: SuiteReport): Column {
  const s = r.summary;
  return {
    id: r.id,
    name: r.name,
    team: runLine(r.team),
    passed: s.passed,
    scored: s.scored,
    total: s.runs,
    rate: s.pass_rate,
    range: wilson(s.passed, s.scored),
    runs: r.runs.map(cell),
    checksMean: s.checks_passed.mean,
    checksTotal: Math.max(0, ...r.runs.map((x) => x.outcome?.checks_total ?? 0)),
    cost: s.cost_usd.mean,
    time: s.measured_s.mean,
    violations: s.safety_violations,
    denials: s.denials ?? 0,
    flags: s.flags ?? 0,
    unscored: s.unscored,
  };
}

/** A figure compared across the columns. */
export interface Metric {
  id: string;
  label: string;
  /** Which way is better; null for a figure that is neither (more flags is not better by itself). */
  better: 'higher' | 'lower' | null;
  value: (c: Column) => number | null;
  text: (c: Column) => string;
  /** A value worth the alarm colour (a safety violation), not just a worse one. */
  abnormal?: (c: Column) => boolean;
}

export const METRICS: Metric[] = [
  { id: 'checks', label: 'Checks passed', better: 'higher', value: (c) => c.checksMean, text: (c) => (c.checksMean == null ? '–' : `${c.checksMean.toFixed(1)} of ${c.checksTotal}`) },
  { id: 'violations', label: 'Safety violations', better: 'lower', value: (c) => (c.scored ? c.violations : null), text: (c) => (c.scored ? String(c.violations) : '–'), abnormal: (c) => c.violations > 0 },
  { id: 'denials', label: 'Denied actions', better: 'lower', value: (c) => (c.scored ? c.denials : null), text: (c) => (c.scored ? String(c.denials) : '–') },
  { id: 'flags', label: 'Flags for a human', better: null, value: (c) => (c.scored ? c.flags : null), text: (c) => (c.scored ? String(c.flags) : '–') },
  { id: 'cost', label: 'Cost per run', better: 'lower', value: (c) => c.cost, text: (c) => fmt(c.cost, 'usd') },
  { id: 'time', label: 'Time per run', better: 'lower', value: (c) => c.time, text: (c) => duration(c.time) },
];

/** The figures about safety: folded into one line when every setup has none. */
export const SAFETY_METRICS = ['violations', 'denials', 'flags'];

/** True when some run is scored and no setup has a safety violation, a denial or a flag. */
export function safetyQuiet(cols: Column[]): boolean {
  return cols.some((c) => c.scored > 0) && cols.every((c) => !c.violations && !c.denials && !c.flags);
}

/** The columns with the best value of a metric; none when there is one column or all are equal. */
export function bestOf(cols: Column[], m: Metric): Set<number> {
  const vals = cols.map(m.value);
  const known = vals.filter((v): v is number => v != null);
  if (!m.better || cols.length < 2 || known.length < 2) return new Set();
  const top = m.better === 'higher' ? Math.max(...known) : Math.min(...known);
  if (known.every((v) => v === top)) return new Set();
  return new Set(vals.flatMap((v, i) => (v === top ? [i] : [])));
}

/** "no_expired_lot_shipped" → "No expired lot shipped". */
export function checkLabel(name: string): string {
  const words = name.replace(/[_-]+/g, ' ').trim();
  return words ? words[0].toUpperCase() + words.slice(1) : name;
}

export interface CheckCell {
  passed: number;
  runs: number;
}

export interface CheckRow {
  name: string;
  safety: boolean;
  /** One per column; null when the setup's runs have no such check. */
  cells: (CheckCell | null)[];
  /** Every run of every setup passed it. */
  clean: boolean;
}

/** A check's rate in one report, summed over its tasks (a suite from before experiments may have several). */
function rateOf(checks: CheckRate[], name: string): CheckCell | null {
  const of = checks.filter((c) => c.name === name);
  if (!of.length) return null;
  return { passed: of.reduce((n, c) => n + c.passed, 0), runs: of.reduce((n, c) => n + c.runs, 0) };
}

/** Every check of every setup, goal checks first, then safety checks, in the order they first appear. */
export function checkMatrix(reports: SuiteReport[]): CheckRow[] {
  const names: { name: string; safety: boolean }[] = [];
  for (const c of reports.flatMap((r) => r.checks)) if (!names.some((n) => n.name === c.name)) names.push({ name: c.name, safety: c.safety });
  const rows = names.map(({ name, safety }) => {
    const cells = reports.map((r) => rateOf(r.checks, name));
    return { name, safety, cells, clean: cells.every((c) => !c || c.passed === c.runs) };
  });
  return [...rows.filter((r) => !r.safety), ...rows.filter((r) => r.safety)];
}

// ---- New experiment ----

export const MAX_SETUPS = 4;
export const RUN_CHOICES = [1, 3, 5, 10] as const;
export const DEFAULT_RUNS = 5;

/** A Claude Code model for every Claude Code agent of a setup; '' keeps the team's own. */
export const MODELS = [
  { id: '', label: 'As designed' },
  { id: 'haiku', label: 'Haiku' },
  { id: 'sonnet', label: 'Sonnet' },
  { id: 'opus', label: 'Opus' },
] as const;

/** An agent setup being written: a name, which team, a model, and rules every agent follows. */
export interface SetupDraft {
  key: string;
  name: string;
  teamId: string;
  model: string;
  rules: string;
}

export const setupLetter = (i: number) => String.fromCharCode(65 + i);

export function newDraft(i: number, teamId: string, from?: SetupDraft): SetupDraft {
  return { key: `${Date.now()}-${i}`, name: `Setup ${setupLetter(i)}`, teamId, model: from?.model ?? '', rules: from?.rules ?? '' };
}

/**
 * Setups whose team is no longer offered move to the first one. The offered list changes while the
 * page loads (yours arrives first as 'yours', then the benchmark's teams replace that key), and a setup
 * left on a vanished key would never find its team. Returns the same array when nothing needs moving.
 */
export function settleDrafts(drafts: SetupDraft[], keys: string[], first: string | undefined): SetupDraft[] {
  if (!first || !drafts.some((d) => !keys.includes(d.teamId))) return drafts;
  return drafts.map((d) => (keys.includes(d.teamId) ? d : { ...d, teamId: first }));
}

/** The team a setup runs: each Claude Code agent on the chosen model, the rules added to every prompt. */
export function setupTeam(base: Team, d: Pick<SetupDraft, 'model' | 'rules'>): Team {
  return {
    ...base,
    agents: base.agents.map((a) => ({ ...a, model: d.model && a.kind === 'claude_code' ? d.model : a.model, prompt: withRules(a.prompt, d.rules) })),
  };
}

/** Why the setups can't start, or null: a blank or repeated name. */
export function setupsProblem(drafts: SetupDraft[]): string | null {
  const names = drafts.map((d) => d.name.trim());
  if (names.some((n) => !n)) return 'Every setup needs a name.';
  const dup = names.find((n, i) => names.indexOf(n) !== i);
  return dup ? `Two setups are called “${dup}”.` : null;
}
