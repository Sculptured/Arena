import { type LaneRow, type Mark, type Span, formatDuration, position } from './lanes';

const GLYPH: Record<Mark['kind'], string> = {
  action: '', interlock: '◆', message: '→', model: '·', start: '▸', end: '▪', check: '✓', world: '∿',
};

/** Reads hollow, writes solid, interlocks a diamond; errors and refusals the only colour. */
function glyphOf(m: Mark): string {
  if (m.kind !== 'action') return GLYPH[m.kind];
  return m.effect === 'write' ? '■' : m.effect === 'read' ? '○' : '□';
}

function classOf(m: Mark, selected: boolean): string {
  const bits = ['hmi-soe-mark', `k-${m.kind}`];
  if (m.effect === 'write') bits.push('write');
  if (m.outcome !== 'ok') bits.push('abnormal');
  if (m.inherited) bits.push('inherited');
  if (selected) bits.push('selected');
  return bits.join(' ');
}

const offset = (at: number, span: Span) => `+${((at - span.start) / 1000).toFixed(1)} s`;

/** Lanes per agent, each step a mark at its time (the graphic), then the event log (the record). */
export default function ActionTimeline({ lanes, span, selected, onSelect }: {
  lanes: LaneRow[];
  span: Span | null;
  selected: number | null;
  onSelect: (seq: number) => void;
}) {
  if (!span) return <p className="muted">No steps yet.</p>;
  const log = lanes.flatMap((l) => l.marks.map((m) => ({ lane: l.name, m }))).sort((a, b) => a.m.seq - b.m.seq);
  return (
    <div className="hmi-soe">
      {lanes.map((l) => (
        <div key={l.id} className="hmi-lane">
          <div className="hmi-lane-name">{l.name}</div>
          <div className="hmi-track">
            {l.marks.map((m) => (
              <button key={m.seq} className={classOf(m, selected === m.seq)} style={{ left: `${position(m.at, span) * 100}%` }}
                title={`step ${m.seq} · ${m.label}${m.detail ? ` · ${m.detail}` : ''}`} onClick={() => onSelect(m.seq)}>
                {glyphOf(m)}
              </button>
            ))}
          </div>
        </div>
      ))}
      <table className="hmi-table hmi-log">
        <thead><tr><th>Step</th><th>Time</th><th>Agent</th><th>Action</th><th>Detail</th><th>Took</th><th>Outcome</th></tr></thead>
        <tbody>{log.map(({ lane, m }) => (
          <tr key={m.seq} className={selected === m.seq ? 'selected' : m.inherited ? 'inherited' : ''} onClick={() => onSelect(m.seq)}>
            <td className="mono">{m.seq}</td><td className="mono">{Number.isNaN(m.at) ? '–' : offset(m.at, span)}</td>
            <td>{lane}</td>
            <td className={m.effect === 'write' ? 'hmi-write' : ''}><span aria-hidden="true">{glyphOf(m)} </span>{m.label}</td>
            <td className="mono small hmi-ellipsis">{m.detail}</td><td className="mono">{formatDuration(m.durationMs)}</td>
            <td>{m.outcome === 'ok' ? 'ok' : <span className="hmi-mark abnormal">⚠ {m.outcome}</span>}</td>
          </tr>
        ))}</tbody>
      </table>
    </div>
  );
}
