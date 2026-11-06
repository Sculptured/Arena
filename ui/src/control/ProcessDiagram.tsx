import { kindWord, systemLine } from './actions';
import { type Counts, type ProcessView, type UnitState, abnormal, countsLine } from './process';

const ROW = 64;
const STATE: Record<UnitState, { glyph: string; word: string; abnormal: boolean }> = {
  idle: { glyph: '○', word: 'idle', abnormal: false },
  running: { glyph: '◐', word: 'running', abnormal: false },
  done: { glyph: '■', word: 'done', abnormal: false },
  failed: { glyph: '✕', word: 'failed', abnormal: true },
};

function Alarm({ c }: { c: Counts }) {
  if (!abnormal(c)) return null;
  const bits = [c.errors && `${c.errors} error${c.errors > 1 ? 's' : ''}`, c.refused && `${c.refused} refused`].filter(Boolean);
  return <span className="hmi-mark abnormal"><span className="hmi-glyph">⚠</span>{bits.join(', ')}</span>;
}

/** Agents as units (left), the world's systems as equipment (right), and the action flows between them. */
export default function ProcessDiagram({ view }: { view: ProcessView }) {
  const rows = Math.max(view.units.length, view.equipment.length, 1);
  const y = (i: number) => i * ROW + ROW / 2;
  const unitAt = new Map(view.units.map((u, i) => [u.id, i]));
  const partAt = new Map(view.equipment.map((e, i) => [e.part, i]));
  return (
    <div className="hmi-process" style={{ height: rows * ROW }}>
      <div className="hmi-col">
        {view.units.map((u) => {
          const s = STATE[u.state];
          return (
            <div key={u.id} className="hmi-unit" style={{ height: ROW - 8 }}>
              <div className="hmi-unit-name">{u.name}</div>
              <div className="small"><span className={`hmi-mark${s.abnormal ? ' abnormal' : ''}`}><span className="hmi-glyph">{s.glyph}</span>{s.word}</span>
                <span className="mono muted"> {countsLine(u)}</span> <Alarm c={u} /></div>
            </div>
          );
        })}
      </div>
      <svg className="hmi-flows" viewBox={`0 0 100 ${rows * ROW}`} preserveAspectRatio="none" aria-hidden="true">
        {view.flows.map((f) => {
          const [a, b] = [unitAt.get(f.agent), partAt.get(f.part)];
          if (a == null || b == null) return null;
          const cls = `hmi-flow${f.writes ? ' writes' : ''}${abnormal(f) ? ' abnormal' : ''}`;
          return <line key={`${f.agent}/${f.part}`} className={cls} x1={0} y1={y(a)} x2={100} y2={y(b)} vectorEffect="non-scaling-stroke" />;
        })}
      </svg>
      <div className="hmi-col">
        {view.equipment.map((e) => (
          <div key={e.part} className="hmi-equipment" style={{ height: ROW - 8 }} title={systemLine(e.system)}>
            <div><span className="hmi-kind">{kindWord(e.system)}</span> <span className="mono muted small">{e.part}</span>
              {e.system?.simulated && <span className="hmi-tag">simulated</span>}</div>
            <div className="small muted hmi-ellipsis">{systemLine(e.system)}</div>
            <div className="small"><span className="mono muted">{countsLine(e)}</span> <Alarm c={e} /></div>
          </div>
        ))}
      </div>
    </div>
  );
}
