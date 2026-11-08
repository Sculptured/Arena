import { defaultTeam, suggestedTeams } from './api';
import type { AgentSpec, Runner, Team } from './types';

export const TEAM_KEY = 'arena.team.v1';

function valid(t: unknown): t is Team {
  const x = t as Team | null;
  return !!x && typeof x.id === 'string' && typeof x.name === 'string' && Array.isArray(x.agents) && x.agents.length > 0;
}

export function readStoredTeam(): Team | null {
  try {
    const raw = localStorage.getItem(TEAM_KEY);
    if (!raw) return null;
    const t = JSON.parse(raw);
    return valid(t) ? t : null;
  } catch {
    return null;
  }
}

export function saveTeam(team: Team): void {
  try {
    localStorage.setItem(TEAM_KEY, JSON.stringify(team));
  } catch {
    /* storage unavailable: the team lives for this page only */
  }
}

export function clearStoredTeam(): void {
  try {
    localStorage.removeItem(TEAM_KEY);
  } catch {
    /* ignore */
  }
}

/** What makes two teams the same for `isUntouched`: their agents as arenad runs them. */
const sameAgents = (a: Team, b: Team) =>
  JSON.stringify(a.agents.map(({ id, name, kind, model, prompt, read_only }) => [id, name, kind, model, prompt, !!read_only]))
  === JSON.stringify(b.agents.map(({ id, name, kind, model, prompt, read_only }) => [id, name, kind, model, prompt, !!read_only]));

/**
 * True when `stored` is one of the benchmark's suggested teams as it came, never edited nor
 * picked on purpose: a default the browser kept, which follows the benchmark's new default.
 */
export function isUntouched(stored: Team, suggested: Team[]): boolean {
  return !stored.picked && suggested.some((t) => t.id === stored.id && t.name === stored.name && sameAgents(t, stored));
}

/**
 * The stored team, else the server's default. A stored team that is an untouched suggestion other
 * than the default (an old default the browser kept) gives way to the default.
 */
export async function loadTeam(): Promise<Team> {
  const stored = readStoredTeam();
  if (!stored) return defaultTeam();
  const suggested = await suggestedTeams().catch(() => [] as Team[]);
  const first = suggested[0];
  if (first && stored.id !== first.id && isUntouched(stored, suggested.slice(1))) {
    clearStoredTeam();
    return first;
  }
  return stored;
}

/**
 * The team as a run takes it: an open agent's blank provider dropped; with the `external` runner,
 * every agent is `external` (the system that drives the run runs them, not arenad). The board,
 * the suggested runner and the picked mark stay in the browser.
 */
export function teamForRun({ board: _board, runner: _runner, picked: _picked, ...team }: Team, runner: Runner = 'chain'): Team {
  return {
    ...team,
    agents: team.agents.map(({ provider, ...a }) => {
      if (runner === 'external') return { ...a, kind: 'external' as const, model: null };
      const p = provider?.trim();
      return a.kind === 'open' && p ? { ...a, provider: p } : a;
    }),
  };
}

/** Agent ids must match ^[a-z0-9_-]+$ and be unique within the team. */
export function slug(name: string, taken: Set<string>): string {
  const base = name.toLowerCase().replace(/[^a-z0-9_-]+/g, '-').replace(/^-+|-+$/g, '') || 'agent';
  let id = base;
  for (let n = 2; taken.has(id); n++) id = `${base}-${n}`;
  return id;
}

/** Re-derive every id from its name (keeping order, resolving collisions). */
export function withSlugIds(agents: AgentSpec[]): AgentSpec[] {
  const taken = new Set<string>();
  return agents.map((a) => {
    const id = slug(a.name, taken);
    taken.add(id);
    return { ...a, id };
  });
}
