import { defaultTeam } from './api';
import type { AgentSpec, Team } from './types';

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

/** The stored team, else the server's default. */
export async function loadTeam(): Promise<Team> {
  return readStoredTeam() ?? (await defaultTeam());
}

/** The team as a run is started with: a provider only on API-mode agents, trimmed, never blank. */
export function teamForRun(team: Team): Team {
  return {
    ...team,
    agents: team.agents.map(({ provider, ...a }) => {
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
