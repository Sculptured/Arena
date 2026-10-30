import type { Team } from '../types';
import { CHECK_LANE, type Lane, type Step } from './timeline';

const GLYPH: Record<Step['kind'], string> = { start: '▸', model: '◇', tool: '⚙', handoff: '⇢', end: '■', check: '✓' };

export default function Timeline({ lanes, team, selected, onSelect }: {
  lanes: Lane[];
  team: Team;
  selected: number | null;
  onSelect: (seq: number) => void;
}) {
  const name = (id: string) => (id === CHECK_LANE ? 'Check' : team.agents.find((a) => a.id === id)?.name ?? id);
  return (
    <div className="timeline">
      {lanes.map((l) => (
        <div className="lane" key={l.agent}>
          <div className="lane-name" title={l.agent}>{name(l.agent)}</div>
          <div className="lane-steps">
            {l.steps.map((s) => (
              <button
                key={s.seq}
                className={`step step-${s.kind}${s.wrote ? ' wrote' : ''}${s.inherited ? ' inherited' : ''}${selected === s.seq ? ' selected' : ''}`}
                onClick={() => onSelect(s.seq)}
                title={`step ${s.seq}${s.inherited ? ' (inherited from parent)' : ''}${s.wrote ? ' · writes' : ''}`}
              >
                <span className="step-glyph" aria-hidden="true">{GLYPH[s.kind]}</span>
                <span className="step-seq">{s.seq}</span>
                <span className="step-label">{s.label}</span>
                {s.wrote && <span className="step-mark write" title="writes the database">✎</span>}
                {s.snapshot && <span className="step-mark snap" title="snapshot taken">◉</span>}
              </button>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}
