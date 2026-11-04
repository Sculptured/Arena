import type { Cost, Metrics, Overhead } from '../types';

/** What each kind of arena overhead is, in the order they are shown. */
export const OVERHEAD_KINDS: [keyof Overhead, string][] = [
  ['restore', 'world start / restore'],
  ['save', 'saves'],
  ['record', 'recording'],
  ['gateway', 'gateway hop'],
  ['mcp', 'MCP endpoint'],
  ['gate', 'gate checks'],
  ['link', 'link calls'],
  ['wait', 'waiting for the step lock'],
  ['score', 'final save and scoring'],
];

const secs = (s: number) => (s < 10 ? s.toFixed(2) : s.toFixed(1));

/** `[label, "1.23 s"]` for each kind of overhead that took any time, largest first. */
export function overheadLines(o: Overhead): [string, string][] {
  return OVERHEAD_KINDS.filter(([k]) => (o[k] ?? 0) > 0)
    .sort(([a], [b]) => (o[b] ?? 0) - (o[a] ?? 0))
    .map(([k, label]) => [label, `${secs((o[k] ?? 0) / 1000)} s`]);
}

/** A cost as text: dollars with their label (and a list price's date), or the label alone. */
export function costText(c: Cost): string {
  if (c.usd == null || c.label === 'no API charge') return c.label;
  const usd = `$${c.usd.toFixed(c.usd < 0.1 ? 4 : 2)}`;
  const when = c.as_of ? `, prices of ${c.as_of}` : '';
  return `${usd} (${c.label}${when})`;
}

/** The estimate line: production time, or why there is none. */
export function estimateText(m: Metrics): string {
  if (m.estimated_s == null) return 'not estimated: this run was recorded before arena timed its own work';
  return `${secs(m.estimated_s)} s (${secs(m.measured_s)} s measured − ${secs(m.overhead_s)} s arena overhead)`;
}

/** The branch's deployment numbers: time in production, arena's overhead, tokens and cost per agent. */
export default function MetricsPanel({ m }: { m: Metrics }) {
  const lines = overheadLines(m.overhead_ms);
  return (
    <details className="panel pane metrics">
      <summary><span className="label">Estimated in production</span> {estimateText(m)}</summary>
      {lines.length > 0 && (
        <table className="small"><tbody>
          {lines.map(([label, v]) => <tr key={label}><td>{label}</td><td className="mono">{v}</td></tr>)}
        </tbody></table>
      )}
      <table className="small">
        <thead><tr><th>Agent</th><th>Model</th><th>Tokens (in / out)</th><th>Cost</th></tr></thead>
        <tbody>
          {m.agents.map((a, i) => (
            <tr key={`${a.agent}-${i}`}>
              <td className="mono">{a.agent}</td>
              <td className="mono">{a.model ?? '–'}{a.provider ? ` · ${a.provider}` : ''}</td>
              <td className="mono">{a.tokens}{a.prompt_tokens != null ? ` (${a.prompt_tokens} / ${a.completion_tokens ?? 0})` : ''}</td>
              <td title={a.cost.source ?? ''}>{costText(a.cost)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="muted small">Total: {m.tokens} tokens · {costText(m.cost)}. Only this branch's own steps count.</div>
    </details>
  );
}
