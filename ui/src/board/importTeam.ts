// Import a team from a file: a Sculptured design (`TeamDraft`: groups, agents with a job and a
// place on the board, "hands work to" connections), or an Arena team. Every agent stays a real
// model: Claude agents become Claude Code, Codex agents Codex; a kind Arena can't run is refused
// by name, never swapped for something else.
import type { AgentKind, AgentSpec, Team } from '../types';
import type { Board, BoardLink } from './board';

export type Imported = { team: Team; from: 'Sculptured' | 'Arena' } | { error: string };

type Json = Record<string, unknown>;
const obj = (v: unknown): Json => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Json) : {});
const str = (v: unknown): string | null => (typeof v === 'string' && v.trim() ? v.trim() : null);
const num = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? v : 0);
const list = (v: unknown): unknown[] => (Array.isArray(v) ? v : []);

const KIND: Record<string, AgentKind> = { claude: 'claude_code', codex: 'codex' };
const SCULPTURED_KIND_NAME: Record<string, string> = { opencode: 'OpenCode' };
const ARENA_KINDS: AgentKind[] = ['claude_code', 'codex', 'open', 'external'];

/** A Sculptured agent id as an Arena one (`^[a-z0-9_-]+$`), unique in `taken`. */
function arenaId(id: string, taken: Set<string>): string {
  const base = id.toLowerCase().replace(/[^a-z0-9_-]+/g, '-').replace(/^-+|-+$/g, '') || 'agent';
  let out = base;
  for (let n = 2; taken.has(out); n++) out = `${base}-${n}`;
  taken.add(out);
  return out;
}

/** The order the agents run in one after another: each after the agents that hand it work, otherwise as the file lists them. */
export function workOrder<T extends { id: string }>(agents: T[], links: BoardLink[]): T[] {
  const left = [...agents];
  const out: T[] = [];
  while (left.length) {
    const done = new Set(out.map((a) => a.id));
    const waiting = new Set(links.filter((l) => !done.has(l.from) && left.some((a) => a.id === l.from)).map((l) => l.to));
    // A cycle leaves no agent free: take the first in file order.
    const next = left.findIndex((a) => !waiting.has(a.id));
    out.push(...left.splice(Math.max(0, next), 1));
  }
  return out;
}

/** The model an agent of `kind` runs on: its own, else its group's, else the team's (none: the CLI's default). */
function modelFor(a: Json, kind: string, groups: Map<string, Json>, team: Json): string | null {
  const own = str(obj(a.advanced).model);
  const group = str(obj(obj(groups.get(String(a.group_id))).models)[kind]);
  return own ?? group ?? str(obj(team.models)[kind]);
}

function fromSculptured(d: Json): Imported {
  const raw = list(d.agents).map(obj);
  const unsupported = raw.filter((a) => !KIND[String(a.kind)]);
  if (unsupported.length) {
    const kinds = [...new Set(unsupported.map((a) => SCULPTURED_KIND_NAME[String(a.kind)] ?? String(a.kind)))].join(', ');
    return { error: `Arena can't run ${kinds} agents yet: ${unsupported.map((a) => str(a.name) ?? str(a.id)).join(', ')}. Change them to Claude or Codex in Sculptured and export again.` };
  }
  const groups = new Map(list(d.groups).map(obj).map((g) => [String(g.id), g]));
  const taken = new Set<string>();
  const ids = new Map(raw.map((a) => [String(a.id), arenaId(String(a.id), taken)]));
  const links = list(d.connections).map(obj)
    .filter((c) => c.kind === 'hands-work-to' && ids.has(String(c.from)) && ids.has(String(c.to)))
    .map((c) => ({ from: ids.get(String(c.from))!, to: ids.get(String(c.to))! }));
  const agents: AgentSpec[] = raw.map((a) => {
    const kind = String(a.kind);
    const writes = obj(obj(a.advanced).data_access).writes;
    return {
      id: ids.get(String(a.id))!,
      name: str(a.name) ?? String(a.id),
      kind: KIND[kind],
      model: modelFor(a, kind, groups, d),
      prompt: typeof a.job === 'string' ? a.job : '',
      // An agent whose design says it writes nothing gets only the world's read tools.
      read_only: Array.isArray(writes) && writes.length === 0,
    };
  });
  const decides = Object.fromEntries(raw.map((a) => [ids.get(String(a.id))!, list(obj(obj(a.advanced).authority).domains).filter((x): x is string => typeof x === 'string')]).filter(([, d]) => d.length));
  const board: Board = {
    groups: [...groups.values()].map((g) => ({ id: String(g.id), label: str(g.label) ?? String(g.id), x: num(g.x), y: num(g.y), w: num(g.w), h: num(g.h) })),
    at: Object.fromEntries(raw.map((a) => [ids.get(String(a.id))!, { x: num(a.x), y: num(a.y) }])),
    links,
    ...(Object.keys(decides).length ? { decides } : {}),
  };
  const team: Team = { id: arenaId(str(d.id) ?? 'sculptured-team', new Set()), name: str(d.name) ?? 'Imported team', agents: workOrder(agents, links), board };
  return { team, from: 'Sculptured' };
}

function fromArena(d: Json): Imported {
  const agents = list(d.agents).map(obj);
  const bad = agents.find((a) => !str(a.id) || !str(a.name) || !ARENA_KINDS.includes(a.kind as AgentKind) || typeof a.prompt !== 'string');
  if (bad) return { error: `The agent ${str(bad.name) ?? str(bad.id) ?? '(unnamed)'} needs an id, a name, a kind (claude_code, codex or open) and a prompt.` };
  return { team: d as unknown as Team, from: 'Arena' };
}

/** A team from a file's text, or what is wrong with it. */
export function importTeam(text: string): Imported {
  let d: Json;
  try {
    d = obj(JSON.parse(text));
  } catch {
    return { error: "This isn't a JSON file. Import a Sculptured design (.json) or an Arena team." };
  }
  if ('madl' in d) return { error: 'This is a MADL file. Open it in Sculptured and import the team design Sculptured saves instead.' };
  const agents = list(d.agents).map(obj);
  if (!agents.length) return { error: 'There are no agents in this file.' };
  if (agents.some((a) => 'job' in a) && 'connections' in d) return fromSculptured(d);
  if (str(d.id) && str(d.name)) return fromArena(d);
  return { error: 'This file is neither a Sculptured design nor an Arena team.' };
}
