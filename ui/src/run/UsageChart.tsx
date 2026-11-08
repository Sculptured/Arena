import type { Metrics } from '../types';

/** Zero-based, directly labelled bars. Unknown costs remain unknown in MetricsPanel. */
export default function UsageChart({ metrics }: { metrics: Metrics }) {
  const max = Math.max(1, ...metrics.agents.map((agent) => agent.tokens));
  return <section className="usage-chart" aria-label="Token use by agent">
    <div className="activity-heading"><strong>Token use by agent</strong><span className="muted small">Input + output · this branch</span></div>
    {metrics.agents.map((agent, i) => <div className="usage-row" key={`${agent.agent}-${i}`}>
      <span className="mono small">{agent.agent}</span>
      <div className="usage-track"><div className="usage-fill" style={{ width: `${100 * agent.tokens / max}%` }} /></div>
      <strong className="mono small">{agent.tokens.toLocaleString()}</strong>
    </div>)}
    <p className="muted small">Cumulative tokens across calls; this is not the context window of an individual call.</p>
  </section>;
}
