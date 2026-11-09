import { useEffect, useRef, useState } from 'react';
import { Panel, PanelHeader } from '../components/ui/panel';
import { type Candidate, type Study, objectiveText, ticks } from './candidates';

const H = 184;
const M = { l: 40, r: 108, t: 14, b: 24 };

function useWidth() {
  const ref = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(el.clientWidth));
    ro.observe(el);
    setWidth(el.clientWidth);
    return () => ro.disconnect();
  }, []);
  return { ref, width };
}

/** The value scale: every candidate with its interval, the target and the verified value, with some room. */
function yDomain(s: Study): [number, number] {
  const vals = s.candidates.flatMap((c) => [c.value - (c.ci ?? 0), c.value + (c.ci ?? 0)]);
  for (const v of [s.target, s.verified]) if (v != null) vals.push(v);
  const lo = Math.min(...vals);
  const hi = Math.max(...vals);
  const pad = Math.max((hi - lo) * 0.08, 0.5);
  return [lo - pad, hi + pad];
}

const tickText = (t: number, unit: string) => `${Number.isInteger(t) ? t : t.toFixed(1)}${unit === '%' ? '%' : ''}`;

function title(c: Candidate, s: Study): string {
  const ci = c.ci != null ? ` ± ${c.ci.toFixed(1)}` : '';
  const lines = [`Simulation ${c.n} · ${objectiveText(c.value, s.objective.unit)}${ci}`, c.changes || "Today's config"];
  if (!c.feasible) lines.push('Missed a limit');
  return lines.join('\n');
}

/** The chart's scales: simulation number across, the objective up. */
interface Frame { x: (k: number) => number; y: (v: number) => number; pw: number; lo: number; hi: number; xMax: number }

function frameOf(s: Study, width: number): Frame {
  const [lo, hi] = yDomain(s);
  const xMax = Math.max(s.candidates.length, ...s.candidates.map((c) => c.n));
  const pw = Math.max(40, width - M.l - M.r);
  const ph = H - M.t - M.b;
  return { lo, hi, xMax, pw, x: (k) => M.l + ((k - 0.5) / xMax) * pw, y: (v) => M.t + (1 - (v - lo) / (hi - lo)) * ph };
}

function Axes({ s, f }: { s: Study; f: Frame }) {
  const xt = ticks(1, f.xMax, Math.min(6, f.xMax)).filter((t) => Number.isInteger(t) && t >= 1);
  return (
    <>
      {ticks(f.lo, f.hi).map((t) => (
        <g key={t}>
          <line x1={M.l} x2={M.l + f.pw} y1={f.y(t)} y2={f.y(t)} stroke="var(--line)" />
          <text x={M.l - 6} y={f.y(t)} dy="0.35em" textAnchor="end" className="fill-faint text-2xs tabular-nums">{tickText(t, s.objective.unit)}</text>
        </g>
      ))}
      {xt.map((t) => <text key={t} x={f.x(t)} y={H - 6} textAnchor="middle" className="fill-faint text-2xs tabular-nums">{t}</text>)}
    </>
  );
}

/** The target as a dashed line and the verified value as a diamond, labelled on the right; the target's label moves off the verified one when they would touch. */
function Lines({ s, f }: { s: Study; f: Frame }) {
  const vy = s.verified != null ? f.y(s.verified) : null;
  const ty = s.target != null ? f.y(s.target) : null;
  const tyLabel = vy != null && ty != null && Math.abs(ty - vy) < 13 ? vy + (ty >= vy ? 13 : -13) : ty;
  const label = (at: number, text: string, cls: string) => <text x={M.l + f.pw + 8} y={at} dy="0.35em" className={cls}>{text}</text>;
  return (
    <>
      {ty != null && <>
        <line x1={M.l} x2={M.l + f.pw} y1={ty} y2={ty} stroke="var(--line-strong)" strokeDasharray="4 3" />
        {label(tyLabel!, `Target ${objectiveText(s.target, s.objective.unit)}`, 'fill-muted text-2xs')}
      </>}
      {vy != null && <>
        <path d={`M${M.l + f.pw},${vy - 4.5}l4.5,4.5l-4.5,4.5l-4.5,-4.5z`} fill="var(--fg)" />
        {label(vy, `Verified ${objectiveText(s.verified, s.objective.unit)}`, 'fill-fg text-2xs font-semibold')}
      </>}
    </>
  );
}

/** One candidate: its interval, its dot (ink when it set a new best, grey otherwise, hollow when it missed a limit). */
function Dot({ c, s, f, selected, onSelect }: { c: Candidate; s: Study; f: Frame; selected: boolean; onSelect: (seq: number) => void }) {
  const record = c.feasible && c.best === c.value;
  const cx = f.x(c.n);
  const cy = f.y(c.value);
  return (
    <g onClick={() => onSelect(c.seq)} className="cursor-pointer">
      <title>{title(c, s)}</title>
      {c.ci != null && <line x1={cx} x2={cx} y1={f.y(c.value - c.ci)} y2={f.y(c.value + c.ci)} stroke="var(--faint)" strokeOpacity={0.6} />}
      <circle cx={cx} cy={cy} r={9} fill="transparent" />
      {selected && <circle cx={cx} cy={cy} r={6.5} fill="none" stroke="var(--accent)" strokeWidth={2} />}
      <circle cx={cx} cy={cy} r={3.5} strokeWidth={1.25}
        fill={!c.feasible ? 'var(--surface)' : record ? 'var(--fg)' : 'var(--faint)'} stroke={c.feasible ? 'var(--surface)' : 'var(--muted)'} />
    </g>
  );
}

export function Chart({ s, width, selected, onSelect }: { s: Study; width: number; selected: number | null; onSelect: (seq: number) => void }) {
  const f = frameOf(s, width);
  const steps = s.candidates.filter((c) => c.best != null);
  const path = steps.map((c, i) => `${i ? `H${f.x(c.n)}V` : `M${f.x(c.n)},`}${f.y(c.best!)}`).join('') + (steps.length ? `H${f.x(f.xMax + 0.5)}` : '');
  return (
    <svg width={width} height={H} role="img" aria-label={`${s.objective.label} of each of ${s.candidates.length} simulations and the best so far`} className="block select-none">
      <Axes s={s} f={f} />
      <Lines s={s} f={f} />
      <path d={path} fill="none" stroke="var(--fg)" strokeWidth={1.5} />
      {s.candidates.map((c) => <Dot key={c.seq} c={c} s={s} f={f} selected={selected === c.seq} onSelect={onSelect} />)}
    </svg>
  );
}

/** An optimisation run at a glance: each candidate's objective by simulation, and the best feasible one so far. */
export default function BestSoFar({ s, selected, onSelect }: { s: Study; selected: number | null; onSelect: (seq: number) => void }) {
  const { ref, width } = useWidth();
  const best = s.candidates.at(-1)?.best ?? null;
  const n = s.candidates.length;
  return (
    <Panel>
      <PanelHeader title="Best so far" meta={`${s.objective.label} · ${n} simulation${n === 1 ? '' : 's'}`}>
        <span className="text-sm font-semibold tabular-nums">{objectiveText(best, s.objective.unit)}</span>
      </PanelHeader>
      <div ref={ref} className="px-1 pt-2 pb-1">{width > 0 && <Chart s={s} width={width} selected={selected} onSelect={onSelect} />}</div>
    </Panel>
  );
}
