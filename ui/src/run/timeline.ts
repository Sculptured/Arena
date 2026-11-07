import { LOCAL_COST_SOURCE, providerId } from '../providers';
import type { AgentKind, AgentSpec, Branch, Change, Event } from '../types';

export function costLabel(source: string | null | undefined): string {
  if (source === 'api_equivalent_estimate') return 'API-equivalent estimate';
  if (source === 'openrouter') return 'OpenRouter';
  if (source === LOCAL_COST_SOURCE) return 'no API charge';
  if (source === 'mixed') return 'mixed';
  return source ? 'reported' : '';
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

/** Lines added and removed in a diff. */
export function diffCounts(d: DiffLine[]): { added: number; removed: number } {
  return { added: d.filter((l) => l.op === '+').length, removed: d.filter((l) => l.op === '-').length };
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
  const spend = ownSpend(events, branch.start_seq ?? 0);
  const time = elapsed(branch, now);
  return {
    ...time, tokens: spend.tokens, cost_usd: spend.cost,
    cost_source: spend.sources.length === 0 ? null : spend.sources.length === 1 ? spend.sources[0] : 'mixed',
    scope: branch.start_seq != null ? `since fork at step ${branch.start_seq}` : null,
  };
}

/** Tokens and cost from the branch's own `agent_end`s; the cost is unknown once any part is. */
function ownSpend(events: Event[], from: number): { tokens: number; cost: number | null; sources: string[] } {
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
  return { tokens, cost, sources };
}

/** Seconds from the branch's start to its end, or to now while it runs. */
function elapsed(branch: Pick<Branch, 'created_at' | 'ended_at' | 'status'>, now: number): { seconds: number | null; live: boolean } {
  const start = Date.parse(branch.created_at);
  const live = !branch.ended_at && branch.status === 'running';
  const end = branch.ended_at ? Date.parse(branch.ended_at) : live ? now : NaN;
  return { seconds: Number.isNaN(start) || Number.isNaN(end) ? null : (end - start) / 1000, live };
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
    const c = edits[a.id] ? agentChange(a, edits[a.id]) : {};
    if (Object.keys(c).length) out[a.id] = c;
  }
  return out;
}

/** One agent's real changes (see `changedFields`). */
function agentChange(a: AgentSpec, e: Change): Change {
  const c: Change = {};
  if (e.prompt != null && e.prompt.trim() !== a.prompt.trim()) c.prompt = e.prompt.trim();
  const kindChanged = e.kind != null && e.kind !== (a.kind as AgentKind);
  const open = (e.kind ?? a.kind) === 'open';
  const providerChanged = open && e.provider != null && providerId(e.provider) !== providerId(a.provider);
  if (providerChanged) Object.assign(c, { provider: providerId(e.provider), model: e.model?.trim() ?? '' });
  else Object.assign(c, modelChange(a, e.model, kindChanged));
  if (kindChanged) c.kind = e.kind;
  return c;
}

/** A model edit with the provider unchanged: sent when it differs, a blank next to a mode change dropped. */
function modelChange(a: AgentSpec, model: string | null | undefined, kindChanged: boolean): Change {
  const m = model?.trim();
  if (m == null || m === (a.model ?? '') || (m === '' && kindChanged)) return {};
  return { model: m };
}

export const MAX_ROW_KEYS = 50;

/** Row keys to fetch for a step: writes first, wildcard reads (`table/*`) dropped, capped. */
export function rowKeys(writes: string[], reads: string[]): { keys: string[]; more: number } {
  const all = [...new Set([...writes, ...reads])].filter((k) => !k.endsWith('/*'));
  return { keys: all.slice(0, MAX_ROW_KEYS), more: Math.max(0, all.length - MAX_ROW_KEYS) };
}
