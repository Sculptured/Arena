import { Link } from 'react-router-dom';
import { StatusChip } from '../components';
import { Panel } from '../components/ui/panel';
import { cn } from '@/lib/cn';
import type { BranchDetail } from '../types';
import type { TrackMark } from '../world/scrub';
import type { Strip } from './divergence';
import { type Figure, type OutcomeRow, type SideInput, outcomeRows, verdict } from './outcomes';

function Value({ f, better }: { f: Figure; better: boolean }) {
  return <span title={f.title} className={cn('text-sm tabular-nums', f.abnormal && 'font-semibold text-alarm', better && !f.abnormal && 'font-semibold')}>{f.text}</span>;
}

function Row({ r }: { r: OutcomeRow }) {
  const v = verdict(r);
  return (
    <tr className="border-t border-line align-baseline">
      <th scope="row" className="py-2 pr-3 pl-4 text-left text-sm font-medium" title={r.hint}>{r.label}</th>
      <td className="px-2 py-2"><Value f={r.a} better={v === 'a'} /></td>
      <td className="px-2 py-2"><Value f={r.b} better={v === 'b'} /></td>
      <td className="py-2 pr-4 pl-2 text-right text-xs whitespace-nowrap text-muted">{v ? `${v.toUpperCase()} better` : 'same'}</td>
    </tr>
  );
}

function SideHead({ label, d }: { label: string; d: BranchDetail }) {
  return (
    <th scope="col" className="px-2 py-1.5 text-left align-bottom font-normal">
      <span className="flex flex-wrap items-center gap-1.5"><span className="text-xs font-semibold">{label}</span><StatusChip status={d.status} /></span>
      <Link to={`/world/${d.run_id}/${d.id}`} className="block truncate text-2xs text-accent hover:underline" title={d.team.name}>{d.team.name}</Link>
    </th>
  );
}

const GLYPH: Record<TrackMark['kind'], string> = { disruption: '▼', refusal: '▲', fork: '┃' };
const TONE: Record<TrackMark['kind'], string> = { disruption: 'text-warn', refusal: 'text-alarm', fork: 'text-muted' };

const at = (seq: number, s: Strip) => `${s.max > s.min ? ((seq - s.min) / (s.max - s.min)) * 100 : 0}%`;

/** Both branches on one step axis: where they first diverge, each one's disruptions and refusals. */
export function DivergenceStrip({ s }: { s: Strip }) {
  return (
    <div className="flex flex-col gap-1 px-3 pt-1 pb-2">
      <div className="relative ml-6 h-4 text-2xs text-muted">
        {s.first != null && <span className="absolute -translate-x-1/2 font-medium whitespace-nowrap text-fg" style={{ left: at(s.first, s) }}>first divergence · step {s.first}</span>}
      </div>
      {s.rows.map((r) => (
        <div key={r.side} className="flex items-center gap-2">
          <span className="w-4 text-xs font-semibold">{r.side}</span>
          <div className="relative h-6 flex-1">
            <div className="absolute top-3 left-0 h-0.5 bg-line-strong" style={{ width: at(r.end, s) }} />
            {s.first != null && <div className="absolute inset-y-0 w-px bg-fg" style={{ left: at(s.first, s) }} />}
            {r.marks.map((m) => <span key={`${m.kind}${m.seq}`} title={`Step ${m.seq}: ${m.label}`} className={cn('absolute top-0.5 -translate-x-1/2 font-mono text-xs', TONE[m.kind])} style={{ left: at(m.seq, s) }}>{GLYPH[m.kind]}</span>)}
          </div>
        </div>
      ))}
      <div className="relative ml-6 h-4 font-mono text-2xs text-faint">
        {s.ticks.map((t) => <span key={t} className="absolute -translate-x-1/2" style={{ left: at(t, s) }}>{t}</span>)}
      </div>
      <div className="ml-6 flex flex-wrap gap-3 text-2xs text-muted">
        <span><span className="font-mono text-warn">▼</span> disruption</span><span><span className="font-mono text-alarm">▲</span> refused</span><span><span className="font-mono">┃</span> fork</span><span>step →</span>
      </div>
    </div>
  );
}

/** Outcomes, not transcripts: what each branch achieved, side by side. */
export default function OutcomePanel({ a, b }: { a: SideInput; b: SideInput }) {
  const rows = outcomeRows(a, b);
  return (
    <Panel className="overflow-x-auto py-1">
      <table className="w-full min-w-[340px] border-collapse">
        <thead><tr><th scope="col" className="w-[34%]" /><SideHead label="A" d={a.detail} /><SideHead label="B" d={b.detail} /><th scope="col" /></tr></thead>
        <tbody>{rows.map((r) => <Row key={r.id} r={r} />)}</tbody>
      </table>
    </Panel>
  );
}
