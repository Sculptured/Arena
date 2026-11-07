import { useEffect, useRef } from 'react';
import { type LaneRow, type Span, formatDuration } from '../control/lanes';
import { StatusMark } from '../components/ui/status';
import { Table, Td, Th } from '../components/ui/table';
import { cn } from '@/lib/cn';

const offset = (at: number, span: Span | null) => (span && !Number.isNaN(at) ? `+${((at - span.start) / 1000).toFixed(1)} s` : '–');

/** Every step as a row: the record behind the timeline, in step order. */
export default function EventLog({ lanes, span, selected, onSelect, models }: {
  lanes: LaneRow[]; span: Span | null; selected: number | null; onSelect: (seq: number) => void; models: boolean;
}) {
  const box = useRef<HTMLDivElement>(null);
  const rows = lanes.flatMap((l) => l.marks.filter((m) => models || m.kind !== 'model').map((m) => ({ lane: l.name, m }))).sort((a, b) => a.m.seq - b.m.seq);
  useEffect(() => { box.current?.querySelector('[data-active]')?.scrollIntoView({ block: 'nearest' }); }, [selected]);
  return (
    <div ref={box} className="max-h-[420px] overflow-auto">
      <Table className="text-xs" wrapperClassName="overflow-visible">
        <thead className="sticky top-0 z-10"><tr><Th>Step</Th><Th>Time</Th><Th>Agent</Th><Th>Action</Th><Th>Detail</Th><Th className="text-right">Took</Th><Th title="The world was saved after this step">Saved</Th><Th>Outcome</Th></tr></thead>
        <tbody>{rows.map(({ lane, m }) => (
          <tr key={m.seq} data-active={selected === m.seq || undefined} onClick={() => onSelect(m.seq)}
            className={cn('cursor-pointer hover:bg-hover data-[active]:bg-accent-soft', m.inherited && 'opacity-50')}>
            <Td className="h-7 font-mono">{m.seq}</Td><Td className="h-7 font-mono text-muted">{offset(m.at, span)}</Td>
            <Td className="h-7">{lane}</Td>
            <Td className={cn('h-7', m.effect === 'write' && 'font-semibold')}>{m.label}</Td>
            <Td className="h-7 max-w-[28ch] truncate font-mono text-2xs text-muted" title={m.detail}>{m.detail}</Td>
            <Td className="h-7 text-right font-mono text-muted">{formatDuration(m.durationMs)}</Td>
            <Td className="h-7 text-center text-muted">{m.saved && <span title="saved after this step">◉</span>}</Td>
            <Td className="h-7">{m.outcome === 'ok' ? <span className="text-faint">ok</span> : <StatusMark mark={{ glyph: '⚠', word: m.outcome, abnormal: true }} />}</Td>
          </tr>
        ))}</tbody>
      </Table>
    </div>
  );
}
