import { Link } from 'react-router-dom';
import { Tooltip } from '../components/ui/tooltip';
import { cn } from '@/lib/cn';
import { type MeasureRow, measureText, position } from './measures';

/** A dot per scored run on the row's shared scale, with the target as a solid tick and the reference as a dashed one. */
function Dots({ row, i }: { row: MeasureRow; i: number }) {
  const cell = row.cells[i];
  return (
    <div className="relative h-5" role="img" aria-label={`${row.label}: ${cell.dots.map((d) => measureText(d.value, row.unit)).join(', ') || 'no runs yet'}`}>
      <span className="absolute inset-x-0 top-1/2 h-px bg-line" aria-hidden="true" />
      {row.target != null && <span className="absolute inset-y-0 w-px bg-line-strong" style={{ left: `${position(row, row.target)}%` }} aria-hidden="true" />}
      {row.reference && <span className="absolute inset-y-0.5 border-l border-dashed border-faint" style={{ left: `${position(row, row.reference.value)}%` }} aria-hidden="true" />}
      {cell.dots.map((d, k) => {
        const dot = <span className="block size-2.5 rounded-full bg-fg/60 ring-2 ring-surface transition-transform hover:scale-125 hover:bg-fg" />;
        return (
          <Tooltip key={k} content={d.title}>
            <span className="absolute top-1/2 -translate-x-1/2 -translate-y-1/2" style={{ left: `${position(row, d.value)}%` }}>
              {d.href ? <Link to={d.href} aria-label={d.title}>{dot}</Link> : dot}
            </span>
          </Tooltip>
        );
      })}
    </div>
  );
}

/** One measure of one setup: its label, the headline (the best run, or the mean), and the runs as dots. */
export function MeasureFigure({ row, i }: { row: MeasureRow; i: number }) {
  const cell = row.cells[i];
  const word = row.better ? 'best' : 'per run';
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-baseline justify-between gap-3">
        <span className="text-xs text-muted">{row.label}</span>
        <span className="flex items-baseline gap-1.5 tabular-nums">
          {row.better && cell.mean != null && cell.dots.length > 1 && <span className="text-xs text-faint">mean {measureText(cell.mean, row.unit)}</span>}
          <span className={cn('text-lg', cell.best ? 'font-semibold' : 'text-fg/80')}>{measureText(cell.headline, row.unit)}</span>
          {cell.headline != null && <span className="text-xs text-muted">{word}</span>}
        </span>
      </div>
      <Dots row={row} i={i} />
    </div>
  );
}

/** Under the cards, once: what the ticks on the measures' scales are. */
export function MeasureLegend({ rows }: { rows: MeasureRow[] }) {
  const marks = rows.filter((r) => r.target != null || r.reference);
  if (!marks.length) return null;
  return (
    <div className="flex flex-wrap gap-x-6 gap-y-1 px-1 text-xs text-muted">
      {marks.map((r) => (
        <span key={r.name} className="flex flex-wrap items-center gap-x-4 gap-y-1">
          {r.target != null && <span className="flex items-center gap-1.5"><span className="h-3 w-px bg-line-strong" aria-hidden="true" />Target {measureText(r.target, r.unit)}</span>}
          {r.reference && (
            <Tooltip content="Run offline with the same budget and no agent. A number to read the results against, not a run.">
              <span className="flex items-center gap-1.5"><span className="h-3 border-l border-dashed border-faint" aria-hidden="true" />{r.reference.label}: {measureText(r.reference.value, r.unit)}</span>
            </Tooltip>
          )}
        </span>
      ))}
    </div>
  );
}
