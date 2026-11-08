import { type ReactElement, useEffect, useState } from 'react';
import { rows } from '../api';
import { Button } from '../components/ui/button';
import { StatusMark, Tag } from '../components/ui/status';
import { EmptyState, ErrorState } from '../components/ui/state';
import { actionWord, classify, systemLine, worldInfo } from '../control/actions';
import { wakeWord } from '../control/lanes';
import { missingPermit, permitOf } from '../control/permit';
import { notComparedLine, savedLine } from '../restore';
import { costIsUnknown, providerId, providerName, providerOf } from '../providers';
import type { Event, Team } from '../types';
import { formatCost, rowKeys } from './timeline';

export const pretty = (v: unknown) => (typeof v === 'string' ? v : JSON.stringify(v ?? null, null, 2));

type Payload = Record<string, unknown>;
const obj = (v: unknown): Payload => (v && typeof v === 'object' ? (v as Payload) : {});

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="flex flex-col gap-1 border-t border-line px-3 py-2">
      <h3 className="text-2xs font-medium text-muted">{title}</h3>
      {children}
    </section>
  );
}

function Json({ value, error = false }: { value: unknown; error?: boolean }) {
  return <pre className={`max-h-72 overflow-auto rounded-md border border-line bg-sunken p-2 font-mono text-2xs leading-4 whitespace-pre-wrap break-all ${error ? 'text-alarm' : ''}`}>{pretty(value)}</pre>;
}

function Block({ title, value }: { title: string; value: unknown }) {
  return <Section title={title}><Json value={value} /></Section>;
}

function keysOf(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((k): k is string => typeof k === 'string') : [];
}

function useRows(branchId: string, seq: number, keys: string[], auto?: boolean) {
  const [data, setData] = useState<Record<string, unknown> | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const load = async () => {
    setBusy(true); setErr(null);
    try { setData((await rows(branchId, seq, keys)).rows); } catch (e) { setErr(e instanceof Error ? e.message : String(e)); }
    setBusy(false);
  };
  useEffect(() => { setData(null); setErr(null); }, [branchId, seq]);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => { if (auto && keys.length) void load(); }, [auto, branchId, seq]);
  return { data, err, busy, load };
}

function RowsAtStep({ branchId, seq, keys, more, auto }: { branchId: string; seq: number; keys: string[]; more: number; auto?: boolean }) {
  const r = useRows(branchId, seq, keys, auto);
  return (
    <Section title="World rows at this step">
      <div className="flex flex-wrap items-center gap-2 text-xs text-muted">
        <Button size="sm" onClick={r.load} disabled={r.busy || keys.length === 0}>{r.busy ? 'Loading…' : 'Rows at this step'}</Button>
        {keys.length === 0 && <span>nothing read or written</span>}
        {more > 0 && <span>(+{more} more not shown)</span>}
      </div>
      {r.err && <ErrorState error={r.err} hint="The saved world for this step could not be read." />}
      {r.data && Object.entries(r.data).map(([k, doc]) => (
        <div key={k} className="flex flex-col gap-0.5">
          <span className="font-mono text-2xs">{k}{doc === null && <span className="text-muted"> (absent)</span>}</span>
          {doc !== null && <Json value={doc} />}
        </div>
      ))}
    </Section>
  );
}

function KeyList({ label, keys }: { label: string; keys: string[] }) {
  return (
    <div className="flex flex-wrap items-baseline gap-1 text-xs">
      <span className="w-12 text-muted">{label}</span>
      {keys.length ? keys.map((k) => <Tag key={k} className="max-w-full truncate font-mono">{k}</Tag>) : <span className="text-faint">none</span>}
    </div>
  );
}

/** The permit a write ran under (an interlock), or the change it named that approvals never recorded. */
function PermitLine({ events, event }: { events: Event[]; event: Event }) {
  const permit = permitOf(events, event);
  const missing = missingPermit(events, event);
  if (!permit && !missing) return null;
  return (
    <Section title="Permit">
      {permit
        ? <div className="flex flex-wrap items-center gap-1.5 text-xs">
          <StatusMark mark={permit.approved === false ? { glyph: '✕', word: 'Rejected', abnormal: true } : permit.approved ? { glyph: '◆', word: 'Granted', abnormal: false } : { glyph: '◇', word: 'Pending', abnormal: false }} />
          <span className="font-mono text-2xs">{permit.id}</span><span className="text-muted">{permit.decision || permit.action}</span>
        </div>
        : <StatusMark mark={{ glyph: '◇', word: `no permit: ${missing} is not in approvals`, abnormal: true }} />}
    </Section>
  );
}

function ToolCall({ branchId, event, events, auto }: { branchId: string; event: Event; events: Event[]; auto?: boolean }) {
  const p = obj(event.payload);
  const reads = keysOf(p.reads), writes = keysOf(p.writes);
  const { keys, more } = rowKeys(writes, reads);
  return (
    <>
      <PermitLine events={events} event={event} />
      {p.error ? <Section title="Error"><Json value={p.error} error /></Section> : null}
      <Block title="Arguments" value={p.args} />
      <Section title="Reads / writes">
        <KeyList label="Reads" keys={reads} /><KeyList label="Writes" keys={writes} />
        {typeof p.snapshot_error === 'string' && <ErrorState error={`Saving after this step failed: ${p.snapshot_error}`} hint="Reruns from this step can't use a save." />}
        {savedLine(p) && <span className="text-xs text-muted">{savedLine(p)}</span>}
      </Section>
      <RowsAtStep branchId={branchId} seq={event.seq} keys={keys} more={more} auto={auto} />
      {!p.error && <Block title="Result" value={p.result} />}
    </>
  );
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

function ModelCall({ p }: { p: Payload }) {
  const msgs: unknown[] = Array.isArray(obj(p.request).messages) ? (obj(p.request).messages as unknown[]) : [];
  return (
    <>
      <Section title="Provider"><span className="font-mono text-xs">{modelCallLine(p as { provider?: string; request?: unknown })}</span></Section>
      <Section title={`Messages (${msgs.length})`}>
        {msgs.length === 0 && <span className="text-xs text-faint">The request was not recorded.</span>}
        {msgs.map((m, i) => (
          <details key={i} open={i === msgs.length - 1} className="text-xs">
            <summary className="cursor-pointer font-mono text-muted">{(m as { role?: string })?.role ?? '?'}</summary>
            <Json value={messageText(m)} />
          </details>
        ))}
      </Section>
      <Block title="Response" value={p.response} />
      <Block title="Usage" value={p.usage} />
      <Section title="Cost"><span className="text-xs">{formatCost(p.cost_usd as number | null, p.cost_source as string | null, costIsUnknown(providerId(p.provider as string | undefined)))}</span></Section>
      {p.provider_response !== undefined && <Block title="Provider's own response" value={p.provider_response} />}
    </>
  );
}

function Check({ p }: { p: Payload }) {
  const checks = Array.isArray(p.checks) ? (p.checks as { name: string; passed: boolean; safety?: boolean; detail?: string }[]) : [];
  const diff = keysOf(p.differing_rows);
  return (
    <>
      <Section title="Outcome"><StatusMark mark={p.passed ? { glyph: '✓', word: 'Passed', abnormal: false } : { glyph: '✕', word: 'Failed', abnormal: true }} /></Section>
      {checks.length > 0 && (
        <Section title={`Checks (${checks.filter((c) => c.passed).length}/${checks.length})`}>
          <ul className="flex flex-col gap-0.5">{checks.map((c) => (
            <li key={c.name} className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-xs" title={c.detail}>
              <StatusMark mark={c.passed ? { glyph: '✓', word: 'Pass', abnormal: false } : { glyph: '✕', word: 'Fail', abnormal: true }} />
              <span className="font-mono text-2xs">{c.name}</span>{c.safety && <Tag tone={c.passed ? undefined : 'alarm'}>◆ safety</Tag>}
              {!c.passed && c.detail && <span className="w-full pl-6 text-2xs text-muted">{c.detail}</span>}
            </li>
          ))}</ul>
        </Section>
      )}
      {diff.length > 0 && <Block title="Differing rows" value={diff} />}
      {notComparedLine(keysOf(p.unchecked), 'check') && <Section title="Not compared"><span className="text-xs">{notComparedLine(keysOf(p.unchecked), 'check')}</span></Section>}
      {typeof p.gold_db === 'string' && <Section title="Gold"><span className="font-mono text-2xs text-muted">gold db {p.gold_db}</span></Section>}
    </>
  );
}

/** True when the agent is on a provider that reports no price (OpenAI, Anthropic). */
function agentCostUnknown(team: Team | undefined, agent: string | null): boolean {
  const a = team?.agents.find((x) => x.id === agent);
  return !!a && a.kind === 'open' && costIsUnknown(providerOf(a));
}

function AgentEnd({ p, team, agent }: { p: Payload; team?: Team; agent: string | null }) {
  const ok = p.ok !== false;
  const bits = [p.seconds != null ? `${Number(p.seconds).toFixed(1)} s` : '', p.tokens != null ? `${p.tokens} tokens` : '',
    formatCost(p.cost_usd as number | null, p.cost_source as string | null, agentCostUnknown(team, agent))].filter(Boolean);
  return (
    <Section title="Outcome">
      <span className="flex flex-wrap items-center gap-2 text-xs">
        <StatusMark mark={ok ? { glyph: '■', word: 'Finished', abnormal: false } : { glyph: '✕', word: 'Failed', abnormal: true }} />
        {typeof p.error === 'string' && p.error && <span className="text-alarm">{p.error}</span>}
      </span>
      <span className="text-xs text-muted">{bits.join(' · ')}</span>
    </Section>
  );
}

/** Why a dispatch run woke the agent, the steps whose mail it got, and the input it was given. */
function Wake({ p }: { p: Record<string, unknown> }) {
  const mail = Array.isArray(p.mail) ? p.mail.map(String) : [];
  const title = `${wakeWord(p)}${mail.length ? ` (step ${mail.join(', ')})` : ''}: its input`;
  return <Block title={title} value={p.input} />;
}

/** A dispatch run's steps in words: delegations, reports, wakes, waits and the lead's finish. */
const DISPATCH: Partial<Record<Event['kind'], (p: Payload) => ReactElement>> = {
  delegation: (p) => <Block title={`Work given to ${String(p.to)}`} value={p.task} />,
  report: (p) => <Block title={p.turn_end ? `What it said at the end of its turn, passed on to ${String(p.to)} as its report` : `Report to ${String(p.to)}`} value={p.text} />,
  wake: (p) => <Wake p={p} />,
  idle: (p) => <Block title={p.ok === false ? `Its turn failed; it waits: ${String(p.error ?? '')}` : 'Ended its turn and waits for work; it said'} value={p.text} />,
  finish: (p) => <Block title="Finished the run (only the lead can): every agent stopped and the world was scored" value={p.summary} />,
};

export interface InspectorProps {
  branchId: string;
  event: Event | null;
  team?: Team;
  autoRows?: boolean;
  /** The branch's events, for the system an action touched and the permit it ran under. */
  events?: Event[];
}

function Body({ branchId, event, team, autoRows, events = [] }: InspectorProps & { event: Event }) {
  const p = obj(event.payload);
  switch (event.kind) {
    case 'tool_call': return <ToolCall branchId={branchId} event={event} events={events} auto={autoRows} />;
    case 'model_call': return <ModelCall p={p} />;
    case 'handoff': return <Block title={p.to ? `Handoff note for ${String(p.to)}` : 'Handoff note'} value={p.note} />;
    case 'message': return <Block title={`Message to ${p.to === '*' ? 'everyone' : String(p.to)}`} value={p.text} />;
    case 'check': return <Check p={p} />;
    case 'agent_end': return <AgentEnd p={p} team={team} agent={event.agent} />;
    default: return DISPATCH[event.kind]?.(p) ?? <Block title="Payload" value={p} />;
  }
}

/** The step's header: its number, kind, agent and, for an action, the system it touched and how. */
function Head({ event, events = [] }: { event: Event; events?: Event[] }) {
  const a = classify(event, worldInfo(events));
  return (
    <div className="flex flex-col gap-1 px-3 py-2">
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="font-mono text-sm font-semibold">step {event.seq}</span>
        <Tag>{event.kind.replace('_', ' ')}</Tag>
        {event.agent && <span className="text-xs text-muted">{event.agent}</span>}
        {a && <span className="font-mono text-xs">{a.tool}</span>}
        {a && a.outcome !== 'ok' && <StatusMark mark={{ glyph: '⚠', word: a.outcome, abnormal: true }} />}
      </div>
      {a && (
        <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted">
          <Tag className={a.effect === 'write' ? 'border-line-strong font-semibold text-fg' : ''}>{actionWord(a)}</Tag>
          {a.part && <span className="font-mono text-2xs">{a.part}</span>}<span className="truncate">{systemLine(a.system)}</span>
        </div>
      )}
    </div>
  );
}

/** One step in full: what it did, on which system, under which permit, and what it returned. */
export default function Inspector(props: InspectorProps) {
  const { event } = props;
  if (!event) {
    return <EmptyState title="Pick a step on the timeline" />;
  }
  return (
    <div className="min-w-0">
      <Head event={event} events={props.events} />
      <Body {...props} event={event} />
    </div>
  );
}
