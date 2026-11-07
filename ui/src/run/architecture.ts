// "Swap architecture" in the fork panel: another deployment of the same work. Team v2 keeps the
// team's shape and gives every agent its rules (prompt changes); the single agent replaces the
// whole team, which a fork can do only for a run an external system drives.
import type { Change, Event, RunStartPayload, Team } from '../types';

export interface Architecture {
  id: string;
  label: string;
  /** What it changes, in a few words. */
  summary: string;
  /** `rules`: added to every agent's prompt. `team`: the team becomes one agent with these rules. */
  shape: 'rules' | 'team';
  rules: string;
}

const V2_RULES = [
  'Book a shipment only after its cold_chain_shipment change is approved; never book first and ask after.',
  "Before allocating a lot, check its remaining shelf life on arrival against the customer's minimum; never ship a short-dated lot.",
  'Before booking, list the shipments already booked so no order is shipped twice.',
].join('\n');

export const ARCHITECTURES: Architecture[] = [
  { id: 'v2', label: 'Team v2', summary: 'books only after approval, checks shelf life', shape: 'rules', rules: V2_RULES },
  {
    id: 'single', label: 'Single agent', summary: 'one agent does every role, with the v2 rules', shape: 'team',
    rules: `You handle the incident alone, end to end: every role's work.\n${V2_RULES}`,
  },
];

export const architectureOf = (id: string) => ARCHITECTURES.find((a) => a.id === id) ?? ARCHITECTURES[0];

/** Which panel the fork opens with: `change=architecture` in the address picks "Swap architecture". */
export type ForkChoice = 'architecture' | 'agents';

export const forkChoiceOf = (param: string | null): ForkChoice => (param === 'architecture' ? 'architecture' : 'agents');

const MARK = '\n\nDeployment rules:\n';

/** A prompt with the rules in place of any earlier deployment rules. */
export function withRules(prompt: string, rules: string): string {
  const base = prompt.includes(MARK) ? prompt.slice(0, prompt.indexOf(MARK)) : prompt;
  const r = rules.trim();
  return r ? `${base.trimEnd()}${MARK}${r}` : base;
}

/** The changes that give every agent the rules; agents whose prompt would not change are left out. */
export function swapChanges(team: Team, rules: string): Record<string, Change> {
  const out: Record<string, Change> = {};
  for (const a of team.agents) {
    const prompt = withRules(a.prompt, rules);
    if (prompt.trim() !== a.prompt.trim()) out[a.id] = { prompt };
  }
  return out;
}

/** The team as one agent (`operator`) run the way its first agent is, with `rules` for a prompt. */
export function singleTeam(team: Team, rules: string): Team {
  const first = team.agents[0];
  const operator = {
    id: 'operator', name: 'Operations agent', kind: first?.kind ?? 'external', model: first?.model ?? null,
    provider: first?.provider ?? null, read_only: false, prompt: withRules('', rules).trimStart(),
  };
  return { id: `${team.id}-single`, name: `${team.name} → single agent`, agents: [operator] };
}

/** Why the branch's run can't take a whole new team, or null when it can (an external run). */
export function teamSwapProblem(events: Event[]): string | null {
  const rs = events.find((e) => e.kind === 'run_start')?.payload as RunStartPayload | undefined;
  return rs?.runner === 'external' ? null : 'Only a run an external system drives can swap its whole team; pick Team v2.';
}
