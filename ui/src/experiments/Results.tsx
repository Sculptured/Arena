import { useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { Panel } from '../components/ui/panel';
import { Tag } from '../components/ui/status';
import { cn } from '@/lib/cn';
import type { SuiteReport } from '../control/types';
import PassRate from './PassRate';
import RunStrip from './RunStrip';
import { type CheckCell, type CheckRow, type Column, METRICS, bestOf, checkLabel, checkMatrix } from './model';

/** The label column, then one column per setup, never wider than a setup needs. */
const gridOf = (n: number): CSSProperties => ({ gridTemplateColumns: `minmax(120px, 160px) repeat(${n}, minmax(0, 300px))` });

function Row({ label, n, children, className }: { label: ReactNode; n: number; children: ReactNode; className?: string }) {
  return (
    <div className={cn('grid items-center gap-x-6 border-t border-line px-4 py-2.5', className)} style={gridOf(n)}>
      <span className="text-xs text-muted">{label}</span>
      {children}
    </div>
  );
}

function Heads({ cols }: { cols: Column[] }) {
  return (
    <div className="grid gap-x-6 px-4 pt-4 pb-3" style={gridOf(cols.length)}>
      <span />
      {cols.map((c) => (
        <div key={c.id} className="flex min-w-0 flex-col">
          <span className="line-clamp-2 text-sm font-semibold" title={c.name}>{c.name}</span>
          <span className="truncate text-xs text-muted" title={c.team}>{c.team}</span>
        </div>
      ))}
    </div>
  );
}

/** One column per setup: the pass rate, a square per run, then each figure, the best one in bold. */
export function Columns({ cols }: { cols: Column[] }) {
  const n = cols.length;
  return (
    <Panel className="overflow-x-auto">
      <div className="min-w-fit">
        <Heads cols={cols} />
        <Row label="Pass rate" n={n} className="py-4">{cols.map((c) => <PassRate key={c.id} c={c} />)}</Row>
        <Row label="Runs" n={n}>{cols.map((c) => <RunStrip key={c.id} runs={c.runs} />)}</Row>
        {METRICS.map((m) => {
          const best = bestOf(cols, m);
          return (
            <Row key={m.id} label={m.label} n={n}>
              {cols.map((c, i) => (
                <span key={c.id} className={cn('font-mono text-sm tabular-nums', best.has(i) && 'font-semibold', m.abnormal?.(c) && 'font-semibold text-alarm')}>
                  {m.abnormal?.(c) && <span aria-hidden="true">◆ </span>}{m.text(c)}
                </span>
              ))}
            </Row>
          );
        })}
      </div>
    </Panel>
  );
}

function CheckBar({ cell, safety }: { cell: CheckCell | null; safety: boolean }) {
  if (!cell) return <span className="text-xs text-faint">–</span>;
  const rate = cell.runs ? cell.passed / cell.runs : 0;
  const bad = cell.passed < cell.runs;
  return (
    <span className="flex items-center gap-2" title={`${cell.passed} of ${cell.runs} runs passed`}>
      <span className="relative h-1.5 flex-1 overflow-hidden rounded-full bg-sunken">
        <span className={cn('absolute inset-y-0 left-0 rounded-full', bad && safety ? 'bg-alarm' : 'bg-ok')} style={{ width: `${rate * 100}%` }} />
      </span>
      <span className={cn('w-9 text-right font-mono text-xs tabular-nums', bad ? 'font-medium' : 'text-muted', bad && safety && 'text-alarm')}>{cell.passed}/{cell.runs}</span>
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
          <span className="text-xs text-muted">{failing.length ? `${failing.length} of ${rows.length} failed in some run` : `All ${rows.length} passed in every run`}</span>
          {failing.length < rows.length && (
            <button type="button" className="ml-auto text-xs text-muted hover:text-fg" onClick={() => setAll(!all)}>{all ? 'Only the failed ones' : `Show all ${rows.length}`}</button>
          )}
        </div>
        {shown.map((r) => <CheckLine key={r.name} row={r} n={reports.length} />)}
      </div>
    </Panel>
  );
}
