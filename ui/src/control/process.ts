// The process view (L2): the team's agents as units, the world's systems as equipment, and the
// action flows between them, each counted as reads and writes with abnormal ones (errors,
// refusals) apart.
import type { Event, Team } from '../types';
import { type Action, worldInfo } from './actions';
import type { SystemInfo } from './types';

export type UnitState = 'idle' | 'running' | 'done' | 'failed';

export interface Counts {
  reads: number;
  writes: number;
  /** Calls whose effect the record does not say. */
  other: number;
  errors: number;
  refused: number;
  lastAt: number | null;
}

export interface Unit extends Counts {
  id: string;
  name: string;
  state: UnitState;
}

export interface Equipment extends Counts {
  part: string;
  system: SystemInfo | null;
}

export interface Flow extends Counts {
  agent: string;
  part: string;
}

export interface ProcessView {
  units: Unit[];
  equipment: Equipment[];
  flows: Flow[];
}

const empty = (): Counts => ({ reads: 0, writes: 0, other: 0, errors: 0, refused: 0, lastAt: null });

function count(c: Counts, a: Action) {
  if (a.effect === 'read') c.reads += 1;
  else if (a.effect === 'write') c.writes += 1;
  else c.other += 1;
  if (a.outcome === 'error') c.errors += 1;
  if (a.outcome === 'refused') c.refused += 1;
  if (!Number.isNaN(a.at)) c.lastAt = Math.max(c.lastAt ?? a.at, a.at);
}

/** Each agent's state from its last start and end. */
export function unitStates(events: Event[]): Map<string, UnitState> {
  const out = new Map<string, UnitState>();
  for (const e of [...events].sort((a, b) => a.seq - b.seq)) {
    if (!e.agent) continue;
    if (e.kind === 'agent_start') out.set(e.agent, 'running');
    if (e.kind === 'agent_end') out.set(e.agent, (e.payload as { ok?: boolean })?.ok === false ? 'failed' : 'done');
  }
  return out;
}

function getOr<K, V>(m: Map<K, V>, k: K, make: () => V): V {
  if (!m.has(k)) m.set(k, make());
  return m.get(k)!;
}

export function processView(events: Event[], actions: Action[], team: Team): ProcessView {
  const info = worldInfo(events);
  const states = unitStates(events);
  const units = new Map<string, Unit>(team.agents.map((a) => [a.id, { id: a.id, name: a.name, state: states.get(a.id) ?? 'idle', ...empty() }]));
  const equipment = new Map<string, Equipment>(info.parts.map((p) => [p, { part: p, system: info.systems[p] ?? null, ...empty() }]));
  const flows = new Map<string, Flow>();
  for (const a of actions) {
    count(getOr(units, a.agent, () => ({ id: a.agent, name: a.agent, state: states.get(a.agent) ?? 'idle', ...empty() })), a);
    if (a.part == null) continue;
    count(getOr(equipment, a.part, () => ({ part: a.part!, system: a.system, ...empty() })), a);
    count(getOr(flows, `${a.agent}\u0000${a.part}`, () => ({ agent: a.agent, part: a.part!, ...empty() })), a);
  }
  return { units: [...units.values()], equipment: [...equipment.values()], flows: [...flows.values()] };
}

/** True when a flow or unit carried an error or a refusal. */
export const abnormal = (c: Counts) => c.errors > 0 || c.refused > 0;

/** "3 R · 1 W": reads, writes and any calls of unstated effect. */
export function countsLine(c: Counts): string {
  const bits = [`${c.reads} R`, `${c.writes} W`];
  if (c.other) bits.push(`${c.other} ?`);
  return bits.join(' · ');
}
