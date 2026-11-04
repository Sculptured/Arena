import { savedParts } from '../restore';
import { LOCAL_COST_SOURCE, providerId } from '../providers';
import type { AgentKind, AgentSpec, Branch, Change, Event, Team } from '../types';

export type StepKind = 'start' | 'model' | 'tool' | 'handoff' | 'end' | 'check';

export interface Step {
  seq: number;
  kind: StepKind;
  label: string;
  wrote: boolean;
  snapshot: boolean;
  inherited: boolean;
}

export interface Lane {
  agent: string;
  steps: Step[];
}

export const CHECK_LANE = 'check';

const nonEmpty = (v: unknown) => Array.isArray(v) && v.length > 0;

/** One lane per agent in chain order, then a lane for the check. Steps below `startSeq` are inherited. */
export function lanes(events: Event[], team: Team, startSeq: number | null = null): Lane[] {
  const byAgent = new Map<string, Step[]>(team.agents.map((a) => [a.id, []]));
  const checks: Step[] = [];
  for (const e of [...events].sort((a, b) => a.seq - b.seq)) {
    if (e.kind === 'run_start') continue;
    const p = (e.payload ?? {}) as Record<string, unknown>;
    const step: Step = {
      seq: e.seq, kind: 'start', label: '', wrote: false, snapshot: false,
      inherited: startSeq != null && e.seq < startSeq,
    };
    switch (e.kind) {
      case 'agent_start': step.kind = 'start'; step.label = 'start'; break;
      case 'agent_end': step.kind = 'end'; step.label = p.ok === false ? 'end (failed)' : 'end'; break;
      case 'model_call': step.kind = 'model'; step.label = 'model'; break;
      case 'handoff': step.kind = 'handoff'; step.label = typeof p.to === 'string' ? `handoff → ${p.to}` : 'handoff'; break;
      case 'message': step.kind = 'handoff'; step.label = `message → ${String(p.to ?? '?')}`; break;
      case 'tool_call':
        step.kind = 'tool';
        step.label = typeof p.name === 'string' ? p.name : 'tool';
        step.wrote = nonEmpty(p.writes);
        step.snapshot = savedParts(p).length > 0;
        break;
      case 'check': step.kind = 'check'; step.label = p.passed ? 'check passed' : 'check failed'; break;
    }
    if (e.kind === 'check' || e.agent == null) {
      checks.push(step);
      continue;
    }
    if (!byAgent.has(e.agent)) byAgent.set(e.agent, []);
    byAgent.get(e.agent)!.push(step);
  }
  const out: Lane[] = [...byAgent].map(([agent, steps]) => ({ agent, steps }));
  if (checks.length) out.push({ agent: CHECK_LANE, steps: checks });
  return out;
}

export function costLabel(source: string | null | undefined): string {
  if (source === 'api_equivalent_estimate') return 'API-equivalent estimate';
  if (source === 'openrouter') return 'OpenRouter';
  if (source === LOCAL_COST_SOURCE) return 'no API charge';
  if (source === 'mixed') return 'mixed';
  return '';
}

/**
 * A cost with its source. No dollar figure reads "unknown" when an OpenAI or Anthropic call is in
 * it (`unknown`: they report no price), else "tokens only" (Codex); a local server's cost reads
 * "no API charge".
 */
export function formatCost(usd: number | null | undefined, source: string | null | undefined, unknown = false): string {
  if (usd == null) return unknown ? 'unknown' : 'tokens only';
  if (usd === 0 && source === LOCAL_COST_SOURCE) return 'no API charge';
  const l = costLabel(source);
  return `$${usd.toFixed(usd < 0.1 ? 4 : 2)}${l ? ` (${l})` : ''}`;
}

/** Line-based diff of two pretty-printed JSON values (LCS). */
export interface DiffLine { op: ' ' | '+' | '-'; text: string }
export function jsonDiff(a: unknown, b: unknown): DiffLine[] {
  const fmt = (v: unknown) => (v === undefined ? ['(absent)'] : JSON.stringify(v ?? null, null, 2).split('\n'));
  const x = fmt(a), y = fmt(b);
  const n = x.length, m = y.length;
  const t: number[][] = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--) t[i][j] = x[i] === y[j] ? t[i + 1][j + 1] + 1 : Math.max(t[i + 1][j], t[i][j + 1]);
  const out: DiffLine[] = [];
  let i = 0, j = 0;
  while (i < n && j < m) {
    if (x[i] === y[j]) { out.push({ op: ' ', text: x[i] }); i++; j++; }
    else if (t[i + 1][j] >= t[i][j + 1]) out.push({ op: '-', text: x[i++] });
    else out.push({ op: '+', text: y[j++] });
  }
  while (i < n) out.push({ op: '-', text: x[i++] });
  while (j < m) out.push({ op: '+', text: y[j++] });
  return out;
}

export interface Summary {
  seconds: number | null;
  live: boolean;
  tokens: number;
  cost_usd: number | null;
  cost_source: string | null;
  /** "since fork at step N" for a forked branch, else null. */
  scope: string | null;
}

/** Same arithmetic as the server's compare summary: own events only (seq >= start_seq). */
export function summarize(
  branch: Pick<Branch, 'start_seq' | 'created_at' | 'ended_at' | 'status'>,
  events: Event[],
  now: number = Date.now(),
): Summary {
  const from = branch.start_seq ?? 0;
  let tokens = 0;
  let cost: number | null = 0;
  const sources: string[] = [];
  for (const e of events) {
    if (e.seq < from || e.kind !== 'agent_end') continue;
    const p = (e.payload ?? {}) as { tokens?: number | null; cost_usd?: number | null; cost_source?: string | null };
    tokens += p.tokens ?? 0;
    cost = cost != null && p.cost_usd != null ? cost + p.cost_usd : null;
    if (p.cost_source && !sources.includes(p.cost_source)) sources.push(p.cost_source);
  }
  const start = Date.parse(branch.created_at);
  const live = !branch.ended_at && branch.status === 'running';
  const end = branch.ended_at ? Date.parse(branch.ended_at) : live ? now : NaN;
  const seconds = Number.isNaN(start) || Number.isNaN(end) ? null : (end - start) / 1000;
  return {
    seconds, live, tokens, cost_usd: cost,
    cost_source: sources.length === 0 ? null : sources.length === 1 ? sources[0] : 'mixed',
    scope: branch.start_seq != null ? `since fork at step ${branch.start_seq}` : null,
  };
}

/**
 * Only fields that really changed. Prompts compare trimmed. A cleared model that was set is sent as ""
 * (explicit clear); blank on an unset model is no change. A mode change resets the model server-side,
 * so a blank model box next to it sends just the mode. A provider (API mode only) is sent only when
 * it differs from the agent's (absent is `openrouter`), never blank, and always with its model: the
 * old model belongs to the old provider.
 */
export function changedFields(agents: AgentSpec[], edits: Record<string, Change>): Record<string, Change> {
  const out: Record<string, Change> = {};
  for (const a of agents) {
    const e = edits[a.id];
    if (!e) continue;
    const c: Change = {};
    if (e.prompt != null && e.prompt.trim() !== a.prompt.trim()) c.prompt = e.prompt.trim();
    const kindChanged = e.kind != null && e.kind !== (a.kind as AgentKind);
    const open = (e.kind ?? a.kind) === 'open';
    const providerChanged = open && e.provider != null && providerId(e.provider) !== providerId(a.provider);
    const m = e.model?.trim();
    if (providerChanged) {
      c.provider = providerId(e.provider);
      c.model = m ?? '';
    } else if (m != null && m !== (a.model ?? '') && (m !== '' || !kindChanged)) c.model = m;
    if (kindChanged) c.kind = e.kind;
    if (Object.keys(c).length) out[a.id] = c;
  }
  return out;
}

export const MAX_ROW_KEYS = 50;

/** Row keys to fetch for a step: writes first, wildcard reads (`table/*`) dropped, capped. */
export function rowKeys(writes: string[], reads: string[]): { keys: string[]; more: number } {
  const all = [...new Set([...writes, ...reads])].filter((k) => !k.endsWith('/*'));
  return { keys: all.slice(0, MAX_ROW_KEYS), more: Math.max(0, all.length - MAX_ROW_KEYS) };
}
