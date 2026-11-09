import { useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { Panel } from '../components/ui/panel';
import { Tag } from '../components/ui/status';
import { cn } from '@/lib/cn';
import type { SuiteReport } from '../control/types';
import PassRate from './PassRate';
import RunStrip from './RunStrip';
import { MeasureFigure, MeasureLegend } from './Measures';
import type { MeasureRow } from './measures';
import { type CheckCell, type CheckRow, type Column, METRICS, type Metric, SAFETY_METRICS, bestOf, checkLabel, checkMatrix, safetyQuiet } from './model';

/** The checks' label column, then one column per setup. */
const gridOf = (n: number): CSSProperties => ({ gridTemplateColumns: `minmax(140px, 200px) repeat(${n}, minmax(0, 1fr))` });

function Row({ label, n, children, className }: { label: ReactNode; n: number; children: ReactNode; className?: string }) {
  return (
    <div className={cn('grid items-center gap-x-6 border-t border-line px-4 py-2.5', className)} style={gridOf(n)}>
      <span className="text-xs text-muted">{label}</span>
      {children}
    </div>
  );
}

/** Cards side by side, as many columns as setups once there is room. */
const COLS: Record<number, string> = { 1: '', 2: 'sm:grid-cols-2', 3: 'md:grid-cols-3', 4: 'md:grid-cols-2 xl:grid-cols-4' };

function Figure({ m, c, best }: { m: Metric; c: Column; best: boolean }) {
  const bad = !!m.abnormal?.(c);
  return (
    <div className="flex items-baseline justify-between gap-3 py-1.5">
      <dt className="text-xs text-muted">{m.label}</dt>
      <dd className={cn('text-sm tabular-nums', best ? 'font-semibold' : 'text-fg/80', bad && 'font-semibold text-alarm')}>{bad && <span aria-hidden="true">◆ </span>}{m.text(c)}</dd>
    </div>
  );
}

/** One setup: its name and team, the pass rate large, a square per run, any measures, then its figures. */
function SetupCard({ c, i, measures, figures, best, quiet }: { c: Column; i: number; measures: MeasureRow[]; figures: Metric[]; best: (m: Metric) => boolean; quiet: boolean }) {
  return (
    <Panel className="flex flex-col gap-5 p-5">
      <header className="flex min-h-10 flex-col">
        <h2 className="line-clamp-2 text-sm font-semibold" title={c.name}>{c.name}</h2>
        <span className="truncate text-xs text-muted" title={c.team}>{c.team}</span>
      </header>
      <PassRate c={c} />
      <RunStrip runs={c.runs} />
      {measures.length > 0 && <div className="flex flex-col gap-4">{measures.map((r) => <MeasureFigure key={r.name} row={r} i={i} />)}</div>}
      <dl className="flex flex-col border-t border-line pt-2">
        {figures.map((m) => <Figure key={m.id} m={m} c={c} best={best(m)} />)}
        {quiet && <p className="pt-1.5 text-xs text-muted"><span aria-hidden="true">✓ </span>No safety violations, denied actions or flags</p>}
      </dl>
    </Panel>
  );
}

/** A card per setup: the pass rate first, then the runs and measures, then the figures, the best of each in bold. */
export function Setups({ cols, measures = [] }: { cols: Column[]; measures?: MeasureRow[] }) {
  const quiet = safetyQuiet(cols);
  const figures = quiet ? METRICS.filter((m) => !SAFETY_METRICS.includes(m.id)) : METRICS;
  const bests = new Map(figures.map((m) => [m.id, bestOf(cols, m)]));
  return (
    <div className="flex flex-col gap-2">
      <div className={cn('grid gap-4', COLS[cols.length])}>
        {cols.map((c, i) => <SetupCard key={c.id} c={c} i={i} measures={measures} figures={figures} quiet={quiet} best={(m) => !!bests.get(m.id)?.has(i)} />)}
      </div>
      <MeasureLegend rows={measures} />
    </div>
  );
}

function CheckBar({ cell, safety }: { cell: CheckCell | null; safety: boolean }) {
  if (!cell) return <span className="text-xs text-faint">–</span>;
  const rate = cell.runs ? cell.passed / cell.runs : 0;
  const bad = cell.passed < cell.runs;
  return (
    <span className="flex items-center gap-2" title={`${cell.passed} of ${cell.runs} runs passed`}>
      <span className="relative h-1.5 flex-1 overflow-hidden rounded-full bg-sunken">
        <span className={cn('absolute inset-y-0 left-0 rounded-full', !bad ? 'bg-line-strong' : safety ? 'bg-alarm' : 'bg-ok')} style={{ width: `${rate * 100}%` }} />
      </span>
      <span className={cn('w-9 text-right text-xs tabular-nums', bad ? 'font-medium' : 'text-muted', bad && safety && 'text-alarm')}>{cell.passed}/{cell.runs}</span>
    </span>
  );
}

function CheckLine({ row, n }: { row: CheckRow; n: number }) {
  return (
    <Row n={n} className="py-2" label={<span className="flex min-w-0 items-center gap-1.5"><span className="truncate" title={row.name}>{checkLabel(row.name)}</span>{row.safety && <Tag>Safety</Tag>}</span>}>
      {row.cells.map((c, i) => <CheckBar key={i} cell={c} safety={row.safety} />)}
    </Row>
  );
}

/** The checks that some run failed, side by side; every check one click away. */
export function Checks({ reports }: { reports: SuiteReport[] }) {
  const [all, setAll] = useState(false);
  const rows = checkMatrix(reports);
  if (!rows.length) return null;
  const failing = rows.filter((r) => !r.clean);
  const shown = all ? rows : failing;
  return (
    <Panel className="overflow-x-auto">
      <div className="min-w-fit">
        <div className="flex items-center gap-3 px-4 py-3">
          <h2 className="text-sm font-semibold">Checks</h2>
          <span className="text-xs text-muted">{failing.length ? `${failing.length} of ${rows.length} failed in some run` : rows.length === 1 ? 'The check passed in every run' : `All ${rows.length} passed in every run`}</span>
          {failing.length < rows.length && (
            <button type="button" className="ml-auto text-xs text-muted hover:text-fg" onClick={() => setAll(!all)}>{all ? 'Only the failed ones' : `Show all ${rows.length}`}</button>
          )}
        </div>
        {shown.map((r) => <CheckLine key={r.name} row={r} n={reports.length} />)}
      </div>
    </Panel>
  );
}
