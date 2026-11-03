import { agentModelLine } from '../providers';
import type { Team } from '../types';
import { CHECK_LANE, type Lane, type Step } from './timeline';

const GLYPH: Record<Step['kind'], string> = { start: '▸', model: '◇', tool: '⚙', handoff: '⇢', end: '■', check: '✓' };

export default function Timeline({ lanes, team, selected, onSelect }: {
  lanes: Lane[];
  team: Team;
  selected: number | null;
  onSelect: (seq: number) => void;
}) {
  return (
    <div className="timeline">
      {lanes.map((l) => {
        const agent = l.agent === CHECK_LANE ? undefined : team.agents.find((a) => a.id === l.agent);
        const model = agent ? agentModelLine(agent) : null;
        return (
          <div className="lane" key={l.agent}>
            <div className="lane-name" title={model ? `${l.agent} · ${model}` : l.agent}>
              <span className="lane-agent">{l.agent === CHECK_LANE ? 'Check' : agent?.name ?? l.agent}</span>
              {model && <span className="lane-model">{model}</span>}
            </div>
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
                  {s.wrote && <span className="step-mark write" title="writes to the world">✎</span>}
                  {s.snapshot && <span className="step-mark snap" title="saved after this step">◉</span>}
                </button>
              ))}
            </div>
          </div>
        );
      })}
    </div>
  );
}
