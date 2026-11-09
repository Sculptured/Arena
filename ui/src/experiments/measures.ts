// The figures some checks measure (a study's verified objective, the simulations it used), per setup:
// each scored run's value as a dot on a scale every setup shares, and a headline — the best run when
// the measure says which way is better, else the mean. Pure functions only.
import type { SuiteReport } from '../control/types';
import type { Measure } from '../types';

export interface MeasureDot {
  value: number;
  /** Its lane on the strip, so dots that would overlap sit side by side: 0 on the line, then 1, -1, 2, -2. */
  lane: number;
  /** The run's page; null before it started. */
  href: string | null;
  title: string;
}

export interface MeasureCell {
  dots: MeasureDot[];
  /** The best run's value when the measure has a better way, else the mean; null with no value. */
  headline: number | null;
  mean: number | null;
  /** This setup's headline is the best of the setups (only for a measure with a better way). */
  best: boolean;
}

export interface MeasureRow {
  /** The check that measures it. */
  name: string;
  label: string;
  unit: string;
  better: 'higher' | 'lower' | null;
  target: number | null;
  reference: { label: string; value: number } | null;
  /** The scale every setup's dots share. */
  domain: [number, number];
  cells: MeasureCell[];
}

/** "86.4%", "17", "3.2 h". */
export function measureText(v: number | null, unit: string): string {
  if (v == null) return '–';
  const n = Number.isInteger(v) ? String(v) : v.toFixed(1);
  if (!unit) return n;
  return unit === '%' ? `${n}%` : `${n} ${unit}`;
}

const mean = (xs: number[]) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : null);

function cellOf(r: SuiteReport, name: string, better: MeasureRow['better'], unit: string): MeasureCell {
  const dots: MeasureDot[] = [];
  r.runs.forEach((run, i) => {
    const m = run.outcome?.scored ? run.outcome.checks.find((c) => c.name === name)?.measure : undefined;
    if (!m) return;
    dots.push({ value: m.value, lane: 0, href: run.run_id && run.branch_id ? `/runs/${run.run_id}/${run.branch_id}` : null, title: `Run ${i + 1} · ${measureText(m.value, unit)}` });
  });
  const vals = dots.map((d) => d.value);
  const top = !vals.length ? null : better === 'higher' ? Math.max(...vals) : better === 'lower' ? Math.min(...vals) : mean(vals);
  return { dots, headline: top, mean: mean(vals), best: false };
}

/** A scale with room around every value, the target and the reference; counts start at zero. */
function domainOf(values: number[], counts: boolean): [number, number] {
  const lo = counts ? 0 : Math.min(...values);
  const hi = Math.max(...values);
  const pad = Math.max((hi - lo) * 0.08, Math.abs(hi) * 0.01, 0.5);
  return [counts ? 0 : lo - pad, hi + pad];
}

/** Marks the best headline across setups, when the measure has a better way and they differ. */
function markBest(cells: MeasureCell[], better: MeasureRow['better']) {
  const known = cells.map((c) => c.headline).filter((v): v is number => v != null);
  if (!better || known.length < 2) return;
  const top = better === 'higher' ? Math.max(...known) : Math.min(...known);
  if (known.every((v) => v === top)) return;
  for (const c of cells) c.best = c.headline === top;
}

/** The first measure a check gives in any report's runs, by check name, in the order checks first appear. */
function measuresIn(reports: SuiteReport[]): Map<string, Measure> {
  const out = new Map<string, Measure>();
  for (const run of reports.flatMap((r) => r.runs)) {
    for (const c of run.outcome?.checks ?? []) if (c.measure && !out.has(c.name)) out.set(c.name, c.measure);
  }
  return out;
}

const LANES = [0, 1, -1, 2, -2];

/** Puts each dot in the first lane where no dot within `gap` (in value) sits already. */
export function dodge(dots: MeasureDot[], gap: number): void {
  const placed: MeasureDot[] = [];
  for (const d of [...dots].sort((a, b) => a.value - b.value)) {
    d.lane = LANES.find((l) => !placed.some((p) => p.lane === l && Math.abs(p.value - d.value) < gap)) ?? 0;
    placed.push(d);
  }
}

/** One row per measured check, a cell per setup, on a shared scale. */
export function measureRows(reports: SuiteReport[]): MeasureRow[] {
  return [...measuresIn(reports)].map(([name, m]) => {
    const better = m.better ?? null;
    const unit = m.unit ?? '';
    const cells = reports.map((r) => cellOf(r, name, better, unit));
    markBest(cells, better);
    const marks = [...cells.flatMap((c) => c.dots.map((d) => d.value)), ...(m.target != null ? [m.target] : []), ...(m.reference ? [m.reference.value] : [])];
    const domain = domainOf(marks, !better);
    for (const c of cells) dodge(c.dots, (domain[1] - domain[0]) * 0.03);
    return { name, label: m.label, unit, better, target: m.target ?? null, reference: m.reference ?? null, domain, cells };
  });
}

/** Where `v` sits on the row's scale, 0 to 100. */
export function position(row: Pick<MeasureRow, 'domain'>, v: number): number {
  const [lo, hi] = row.domain;
  return hi > lo ? Math.min(100, Math.max(0, ((v - lo) / (hi - lo)) * 100)) : 50;
}
