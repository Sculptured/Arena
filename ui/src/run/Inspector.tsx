import { useEffect, useState } from 'react';
import { rows } from '../api';
import { formatCost, rowKeys } from './timeline';
import { notComparedLine, savedLine } from '../restore';
import { costIsUnknown, providerId, providerName, providerOf } from '../providers';
import type { Event, Team } from '../types';

export const pretty = (v: unknown) => (typeof v === 'string' ? v : JSON.stringify(v ?? null, null, 2));

function Block({ title, value }: { title: string; value: unknown }) {
  return (
    <div className="block">
      <div className="label">{title}</div>
      <pre className="json">{pretty(value)}</pre>
    </div>
  );
}

function keysOf(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((k): k is string => typeof k === 'string') : [];
}

function RowsAtStep({ branchId, seq, keys, more, auto }: { branchId: string; seq: number; keys: string[]; more: number; auto?: boolean }) {
  const [data, setData] = useState<Record<string, unknown> | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => { setData(null); setErr(null); }, [branchId, seq]);
  const load = async () => {
    setBusy(true); setErr(null);
    try { setData((await rows(branchId, seq, keys)).rows); } catch (e) { setErr(e instanceof Error ? e.message : String(e)); }
    setBusy(false);
  };
  useEffect(() => { if (auto && keys.length) void load(); }, [auto, branchId, seq]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <div className="block">
      <button className="btn btn-sm" onClick={load} disabled={busy || keys.length === 0}>
        {busy ? 'Loading…' : 'Rows at this step'}
      </button>
      {keys.length === 0 && <span className="muted small"> nothing read or written</span>}
      {more > 0 && <span className="muted small"> (+{more} more not shown)</span>}
      {err && <div className="notice error">{err}</div>}
      {data && Object.entries(data).map(([k, doc]) => (
        <div key={k} className="row-doc">
          <div className="mono small">{k}{doc === null && <span className="muted"> (absent)</span>}</div>
          {doc !== null && <pre className="json">{pretty(doc)}</pre>}
        </div>
      ))}
    </div>
  );
}

/** True when the agent is on a provider that reports no price (OpenAI, Anthropic). */
function agentCostUnknown(team: Team | undefined, agent: string | null): boolean {
  const a = team?.agents.find((x) => x.id === agent);
  return !!a && a.kind === 'open' && costIsUnknown(providerOf(a));
}

function messageText(m: unknown): string {
  const c = (m as { content?: unknown })?.content;
  return typeof c === 'string' ? c : pretty(c ?? m);
}

/** "OpenAI · gpt-x": who answered a model call (absent in calls recorded before providers: OpenRouter). */
export function modelCallLine(p: { provider?: string; request?: unknown }): string {
  const model = (p.request as { model?: unknown } | null)?.model;
  return `${providerName(providerId(p.provider))}${typeof model === 'string' && model ? ` · ${model}` : ''}`;
}

export default function Inspector({ branchId, event, team, autoRows }: { branchId: string; event: Event | null; team?: Team; autoRows?: boolean }) {
  const auto = autoRows;
  if (!event) return <div className="inspector empty-pane muted">Select a step. Use ↑/↓ or j/k to move.</div>;
  const p = (event.payload ?? {}) as Record<string, any>;
  let body: React.ReactNode;
  switch (event.kind) {
    case 'tool_call': {
      const reads = keysOf(p.reads), writes = keysOf(p.writes);
      const { keys, more } = rowKeys(writes, reads);
      body = (
        <>
          <Block title="Arguments" value={p.args} />
          <div className="block">
            <div className="label">Reads / writes</div>
            <div className="small">reads: {reads.length ? reads.map((k) => <span className="chip mono" key={k}>{k}</span>) : <span className="muted">none</span>}</div>
            <div className="small">writes: {writes.length ? writes.map((k) => <span className="chip mono" key={k}>{k}</span>) : <span className="muted">none</span>}</div>
            {p.snapshot_error && <div className="notice error">Saving after this step failed: {p.snapshot_error}</div>}
            {savedLine(p) && <div className="muted small">{savedLine(p)}</div>}
          </div>
          <RowsAtStep branchId={branchId} seq={event.seq} keys={keys} more={more} auto={auto} />
          {p.error ? <div className="block"><div className="label">Error</div><pre className="json err">{p.error}</pre></div> : <Block title="Result" value={p.result} />}
        </>
      );
      break;
    }
    case 'model_call': {
      const msgs: unknown[] = Array.isArray(p.request?.messages) ? p.request.messages : [];
      body = (
        <>
          <div className="small"><span className="label">Provider</span> <span className="mono">{modelCallLine(p)}</span></div>
          <div className="block">
            <div className="label">Messages ({msgs.length})</div>
            {msgs.map((m, i) => (
              <details key={i} open={i === msgs.length - 1}>
                <summary className="small"><span className="mono">{(m as { role?: string })?.role ?? '?'}</span></summary>
                <pre className="json">{messageText(m)}</pre>
              </details>
            ))}
          </div>
          <Block title="Response" value={p.response} />
          <Block title="Usage" value={p.usage} />
          <div className="muted small">cost: {formatCost(p.cost_usd, p.cost_source, costIsUnknown(providerId(p.provider)))}</div>
          {p.provider_response !== undefined && <Block title="Provider's own response" value={p.provider_response} />}
        </>
      );
      break;
    }
    case 'handoff': body = <Block title={p.to ? `Handoff note for ${p.to}` : 'Handoff note'} value={p.note} />; break;
    case 'message': body = <Block title={`Message to ${p.to === '*' ? 'everyone' : p.to}`} value={p.text} />; break;
    case 'check':
      body = (
        <>
          <div>{p.passed ? 'Passed' : 'Failed'}</div>
          {p.differing_rows?.length ? <Block title="Differing rows" value={p.differing_rows} /> : null}
          {notComparedLine(p.unchecked, 'check') && <div className="small">{notComparedLine(p.unchecked, 'check')}</div>}
          {p.gold_db && <div className="muted small">gold db {String(p.gold_db)}</div>}
        </>
      );
      break;
    case 'agent_end':
      body = (
        <>
          <div>{p.ok ? 'Finished' : 'Failed'}{p.error ? `: ${p.error}` : ''}</div>
          <div className="muted small">{p.seconds != null ? `${Number(p.seconds).toFixed(1)} s · ` : ''}{p.tokens != null ? `${p.tokens} tokens · ` : ''}{formatCost(p.cost_usd, p.cost_source, agentCostUnknown(team, event.agent))}</div>
        </>
      );
      break;
    default: body = <Block title="Payload" value={p} />;
  }
  return (
    <div className="inspector">
      <div className="insp-head">
        <span className="mono">step {event.seq}</span>
        <span className="chip">{event.kind.replace('_', ' ')}</span>
        {event.agent && <span className="muted small">{event.agent}</span>}
        {event.kind === 'tool_call' && <span className="mono">{p.name}</span>}
      </div>
      {body}
    </div>
  );
}
