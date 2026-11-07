import { Table, Td, Th, Tr } from '../components/ui/table';
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
  ['pause', 'paused'],
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

function OverheadTable({ lines }: { lines: [string, string][] }) {
  if (!lines.length) return null;
  return (
    <Table className="text-xs">
      <thead><tr><Th>Arena overhead</Th><Th className="text-right">Time</Th></tr></thead>
      <tbody>{lines.map(([label, v]) => <Tr key={label}><Td>{label}</Td><Td className="text-right font-mono">{v}</Td></Tr>)}</tbody>
    </Table>
  );
}

function AgentsTable({ m }: { m: Metrics }) {
  return (
    <Table className="text-xs">
      <thead><tr><Th>Agent</Th><Th>Model</Th><Th className="text-right">Tokens (in / out)</Th><Th>Cost</Th></tr></thead>
      <tbody>
        {m.agents.map((a, i) => (
          <Tr key={`${a.agent}-${i}`}>
            <Td className="font-mono">{a.agent}</Td>
            <Td className="font-mono">{a.model ?? '–'}{a.provider ? ` · ${a.provider}` : ''}</Td>
            <Td className="text-right font-mono">{a.tokens}{a.prompt_tokens != null ? ` (${a.prompt_tokens} / ${a.completion_tokens ?? 0})` : ''}</Td>
            <Td title={a.cost.source ?? ''}>{costText(a.cost)}</Td>
          </Tr>
        ))}
      </tbody>
    </Table>
  );
}

/** The branch's deployment numbers: time in production, arena's overhead, tokens and cost per agent. */
export default function MetricsPanel({ m }: { m: Metrics }) {
  return (
    <div className="flex flex-col gap-3 p-3">
      <p className="text-sm"><span className="text-2xs font-medium tracking-wide text-muted uppercase">Estimated in production</span> <span className="font-mono">{estimateText(m)}</span></p>
      <div className="grid gap-3 xl:grid-cols-[minmax(0,1fr)_minmax(0,2fr)]">
        <div className="min-w-0 rounded-md border border-line"><OverheadTable lines={overheadLines(m.overhead_ms)} /></div>
        <div className="min-w-0 rounded-md border border-line"><AgentsTable m={m} /></div>
      </div>
      <p className="text-xs text-muted">Total: {m.tokens} tokens · {costText(m.cost)}. Only this branch's own steps count.</p>
    </div>
  );
}
