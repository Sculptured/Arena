// Bring your own agent: the Connect screen's form, the handshake arenad answers
// (`POST /api/agents/check`), the one-agent team made of its answer, and the connected agents
// this browser keeps. A header value is a secret: it goes to arenad once, in the check, and only
// the reference arenad answers with is ever kept or sent again.
import { hostOf, isBedrock, modelOf } from '../providers';
import type { AgentSpec, SecretRef, Team } from '../types';

export type ConnectKind = 'http' | 'bedrock';

export interface ConnectForm {
  kind: ConnectKind;
  url: string;
  /** One `Name: value` per line. */
  headers: string;
  model: string;
  region: string;
}

export const DEFAULT_REGION = 'us-east-1';

export const emptyForm = (kind: ConnectKind = 'http'): ConnectForm => ({ kind, url: '', headers: '', model: '', region: DEFAULT_REGION });

export type CheckBody =
  | { kind: 'http'; url: string; headers?: Record<string, string> }
  | { kind: 'bedrock'; model_id: string; region: string };

/** arenad's answer to the check: the agent as it will store it (header values as references), or why not. */
export type CheckAnswer =
  | { ok: true; agent: AgentSpec; name?: string; ms: number }
  | { ok: false; error: string; ms?: number };

const TOKEN = /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/;

/** The headers text as a map, or the first line that isn't `Name: value`. */
export function parseHeaders(text: string): { headers: Record<string, string> } | { error: string } {
  const headers: Record<string, string> = {};
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (!line) continue;
    const at = line.indexOf(':');
    const name = at > 0 ? line.slice(0, at).trim() : '';
    const value = at > 0 ? line.slice(at + 1).trim() : '';
    if (!name || !TOKEN.test(name) || !value) return { error: `Header line ${i + 1} isn't “Name: value”.` };
    if (Object.keys(headers).some((k) => k.toLowerCase() === name.toLowerCase())) return { error: `The header ${name} is there twice.` };
    headers[name] = value;
  }
  return { headers };
}

/** Why the form can't be tested yet, or null. */
export function formProblem(f: ConnectForm): string | null {
  if (f.kind === 'bedrock') {
    if (!f.model.trim()) return 'Name a Bedrock model.';
    if (!/^[a-z]{2}(-[a-z]+)+-\d+$/.test(f.region.trim())) return 'Name an AWS region, like us-east-1.';
    return null;
  }
  const url = f.url.trim();
  if (!url) return "Give the agent's URL.";
  let u: URL;
  try { u = new URL(url); } catch { return 'The URL should start with https:// or http://.'; }
  if (u.protocol !== 'https:' && u.protocol !== 'http:') return 'The URL should start with https:// or http://.';
  const h = parseHeaders(f.headers);
  return 'error' in h ? h.error : null;
}

/** The check's body; null while the form has a problem. */
export function checkBody(f: ConnectForm): CheckBody | null {
  if (formProblem(f)) return null;
  if (f.kind === 'bedrock') return { kind: 'bedrock', model_id: f.model.trim(), region: f.region.trim() };
  const h = parseHeaders(f.headers);
  const headers = 'headers' in h ? h.headers : {};
  return Object.keys(headers).length ? { kind: 'http', url: f.url.trim(), headers } : { kind: 'http', url: f.url.trim() };
}

const isRef = (v: unknown): v is SecretRef => !!v && typeof v === 'object' && typeof (v as SecretRef).secret === 'string';

/** The agent with every header value that isn't a reference dropped: what this browser may keep. */
export function withoutSecrets(a: AgentSpec): AgentSpec {
  if (!a.headers) return a;
  const headers = Object.fromEntries(Object.entries(a.headers).filter(([, v]) => isRef(v)).map(([k, v]) => [k, { secret: (v as SecretRef).secret }]));
  return { ...a, headers };
}

/** The agent as connected: arenad's, named by what it called itself (else arenad's name, else the host or model). */
export function connectedAgent(answer: Extract<CheckAnswer, { ok: true }>): AgentSpec {
  const a = withoutSecrets(answer.agent);
  const fallback = a.kind === 'http' ? hostOf(a.url) : a.model ?? 'Bedrock agent';
  return { ...a, name: answer.name?.trim() || a.name?.trim() || fallback };
}

/** What makes two connections the same agent: its URL, or its Bedrock model and region. */
export function identity(a: AgentSpec): string {
  if (a.kind === 'http') return `http:${a.url?.trim() ?? ''}`;
  return `${a.provider ?? ''}:${a.model ?? ''}`;
}

const slugOf = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 48) || 'agent';

/** A one-agent team made of a connected agent, run one after another. */
export function connectedTeam(a: AgentSpec): Team {
  const what = a.kind === 'http' ? hostOf(a.url) : `${a.provider ?? ''}-${a.model ?? ''}`;
  return { id: `connected-${slugOf(what)}`, name: a.name, agents: [a], runner: 'chain' };
}

/** True for an agent Connect made: an HTTP agent or a Bedrock model. */
export const isConnectable = (a: AgentSpec) => a.kind === 'http' || (a.kind === 'open' && isBedrock(a.provider));

// ---- the connected agents, kept in this browser ----

export const CONNECTED_KEY = 'arena.connected.v1';

const validAgent = (x: unknown): x is AgentSpec => {
  const a = x as AgentSpec | null;
  return !!a && typeof a.id === 'string' && typeof a.name === 'string' && isConnectable(a);
};

/** The connected agents, newest first (none when storage is unavailable or holds something else). */
export function readConnected(): AgentSpec[] {
  try {
    const raw = localStorage.getItem(CONNECTED_KEY);
    const list: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(list) ? list.filter(validAgent).map(withoutSecrets) : [];
  } catch {
    return [];
  }
}

/** Keeps `agent` first (replacing an earlier connection to the same agent); returns the new list. */
export function saveConnected(agent: AgentSpec): AgentSpec[] {
  const kept = withoutSecrets(agent);
  const list = [kept, ...readConnected().filter((a) => identity(a) !== identity(kept))].slice(0, 12);
  try {
    localStorage.setItem(CONNECTED_KEY, JSON.stringify(list));
  } catch {
    /* storage unavailable: the agent is used now, but not listed next time */
  }
  return list;
}

/** The connected agents as teams, newest first. */
export const connectedTeams = (): Team[] => readConnected().map(connectedTeam);

/** "Plant planner (connected)"; with where it runs when another connected agent has the same name. */
export function connectedLabel(t: Team, all: Team[]): string {
  const twin = all.some((o) => o !== t && o.name === t.name);
  return twin ? `${t.name} (connected, ${modelOf(t.agents[0])})` : `${t.name} (connected)`;
}

/** Where Connect was opened from, and what to give back: that page's form, and which setup gets the agent. */
export interface ConnectFrom {
  from: string;
  setup?: number;
  form?: unknown;
}

/** What Connect gives back: the connected team (its agent as kept: references, never header values), with the page's own state. */
export interface ConnectReturn {
  connected?: Team;
  setup?: number;
  form?: unknown;
}
