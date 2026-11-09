import { cn } from '@/lib/cn';
import { EmptyState } from '../components/ui/state';
import type { Event } from '../types';
import { type Trace, type TraceSpan, spanLength, traceOf } from './waterfall';

/** A bar's fill: the turn neutral, a step in the alarm colour when abnormal, writes darker than reads, arena's own work lightest. */
function barClass(s: TraceSpan): string {
  if (s.kind === 'turn') return 'bg-line-strong';
  if (s.abnormal) return 'bg-alarm';
  if (s.kind === 'overhead') return 'bg-line';
  return s.write ? 'bg-flow-strong' : 'bg-flow';
}

function Label({ s }: { s: TraceSpan }) {
  return (
    <span className="flex min-w-0 items-center gap-1.5" style={{ paddingLeft: s.depth * 12 }}>
      {s.abnormal && <span aria-hidden="true" className="text-alarm">⚠</span>}
      <span className={cn('truncate', s.kind === 'overhead' && 'text-faint', s.kind === 'turn' && 'font-medium', s.abnormal && 'text-alarm')} title={s.detail || s.label}>{s.label}</span>
    </span>
  );
}

function Row({ s, total, selected, onSelect }: { s: TraceSpan; total: number; selected: boolean; onSelect: (seq: number) => void }) {
  const left = (s.start / total) * 100;
  const width = Math.max(0.4, ((s.end - s.start) / total) * 100);
  const small = s.kind === 'overhead';
  return (
    <div className={cn('grid grid-cols-[minmax(0,42%)_1fr_64px] items-center gap-2 px-3 text-xs', s.seq != null && 'cursor-pointer hover:bg-hover', selected && 'bg-accent-soft')}
      style={{ height: small ? 20 : 26 }} onClick={() => s.seq != null && onSelect(s.seq)}>
      <Label s={s} />
      <span className="relative h-full">
        <span className={cn('absolute top-1/2 -translate-y-1/2 rounded-[1px]', barClass(s), small ? 'h-1.5' : 'h-2.5')} style={{ left: `${left}%`, width: `${width}%`, minWidth: 2 }} />
      </span>
      <span className={cn('text-right font-mono text-2xs', small ? 'text-faint' : 'text-muted')}>{s.end - s.start > 0 ? spanLength(s.end - s.start) : 'instant'}</span>
    </div>
  );
}

function Axis({ trace }: { trace: Trace }) {
  return (
    <div className="grid grid-cols-[minmax(0,42%)_1fr_64px] gap-2 border-b border-line px-3 py-1 font-mono text-2xs text-faint">
      <span className="truncate font-sans">{trace.spans.length - 1} rows · time from the turn's start</span>
      <span className="flex justify-between"><span>0</span><span>{spanLength(trace.total / 2)}</span><span>{spanLength(trace.total)}</span></span>
      <span className="text-right">took</span>
    </div>
  );
}

/** The selected step's turn as a trace waterfall: the turn, its steps, and arena's own work in each. */
export default function Waterfall({ events, selected, onSelect }: { events: Event[]; selected: number | null; onSelect: (seq: number) => void }) {
  const trace = selected == null ? null : traceOf(events, selected);
  if (selected == null) return <EmptyState title="Select a step to see its turn" hint="Click a mark on the timeline, or move with ←/→ (j/k). The waterfall shows the model call and every step it led to." />;
  if (!trace) return <EmptyState title="This step belongs to no agent's turn" hint="The check and the world's own events have no turn. Pick an agent's step." />;
  return (
    <div className="min-w-0 py-1">
      <Axis trace={trace} />
      {trace.spans.map((s, i) => <Row key={`${s.seq}-${i}`} s={s} total={trace.total} selected={s.seq != null && s.seq === selected} onSelect={onSelect} />)}
      <p className="px-3 pt-1.5 text-2xs text-faint">A bar ends when the step was recorded. Shakedown's own work in a step is listed under it; where it falls inside the step is not recorded. Gaps are waiting or unseen work.</p>
    </div>
  );
}
