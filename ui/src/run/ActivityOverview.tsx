import type { Lane } from './timeline';

/** One shared sequence axis: positions show event order, never elapsed duration. */
export default function ActivityOverview({ lanes, selected, onSelect }: {
  lanes: Lane[]; selected: number | null; onSelect: (seq: number) => void;
}) {
  const steps = lanes.flatMap((lane) => lane.steps);
  const min = Math.min(...steps.map((step) => step.seq));
  const max = Math.max(...steps.map((step) => step.seq));
  if (!steps.length) return <p className="muted">Waiting for recorded activity.</p>;
  return <section className="activity-overview" aria-label="Agent activity by recorded step">
    <div className="activity-heading"><strong>Agent activity</strong><span className="muted small">Recorded order · select a step to inspect</span></div>
    <div className="activity-axis"><span>Step {min}</span><span>{max}</span></div>
    {lanes.map((lane) => <div className="activity-lane" key={lane.agent}>
      <span className="mono small" title={lane.agent}>{lane.agent}</span>
      <div className="activity-track">
        {lane.steps.map((step) => <button key={step.seq}
          className={`activity-tick activity-${step.kind}${selected === step.seq ? ' selected' : ''}`}
          style={{ left: `${100 * (step.seq - min) / Math.max(1, max - min)}%` }}
          aria-label={`${lane.agent}, step ${step.seq}: ${step.label}`}
          aria-pressed={selected === step.seq}
          title={`${lane.agent} · ${step.seq} · ${step.label}${step.wrote ? ' · writes world state' : ''}`}
          onClick={() => onSelect(step.seq)} />)}
      </div>
      <span className="muted small">{lane.steps.length} steps</span>
    </div>)}
    <p className="muted small">Model calls · tools · handoffs · checks. Spacing represents recorded step order, not time.</p>
  </section>;
}
