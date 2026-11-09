import { Link } from 'react-router-dom';
import { Tooltip } from '../components/ui/tooltip';
import { cn } from '@/lib/cn';
import type { RunCell, RunState } from './model';

const CELL: Record<RunState, string> = {
  passed: 'bg-ok border-ok',
  failed: 'bg-alarm-soft border-alarm text-alarm',
  running: 'border-line-strong bg-sunken animate-pulse',
  pending: 'border-dashed border-line-strong',
  unscored: 'bg-warn-soft border-warn text-warn',
};

const GLYPH: Partial<Record<RunState, string>> = { failed: '✕', unscored: '!' };

function Square({ c }: { c: RunCell }) {
  const body = (
    <span className={cn('relative grid size-5 place-items-center rounded-[3px] border font-mono text-[10px] leading-none font-semibold', CELL[c.state])}>
      {GLYPH[c.state]}
      {c.violations > 0 && <span aria-hidden="true" className="absolute -top-1 -right-1 size-2 rounded-full bg-alarm ring-2 ring-surface" />}
    </span>
  );
  return (
    <Tooltip content={c.title}>
      {c.href ? <Link to={c.href} aria-label={c.title} className="rounded-[3px] transition-transform hover:scale-110">{body}</Link> : <span aria-label={c.title}>{body}</span>}
    </Tooltip>
  );
}

/** One square per run: solid passed, ✕ failed, ! not scored, pulsing while it runs, dashed not run yet; a red dot for a safety violation. */
export default function RunStrip({ runs }: { runs: RunCell[] }) {
  return <div className="flex flex-wrap gap-1">{runs.map((c) => <Square key={c.key} c={c} />)}</div>;
}
