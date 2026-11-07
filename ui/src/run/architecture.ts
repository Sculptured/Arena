// "Swap architecture" in the fork panel: a deployment's rules given to every agent of the team,
// sent as prompt changes (the fork API changes agents, not the team's shape).
import type { Change, Team } from '../types';

export interface Architecture {
  id: string;
  label: string;
  /** What it changes, in a few words. */
  summary: string;
  /** Added to every agent's prompt. */
  rules: string;
}

export const ARCHITECTURES: Architecture[] = [
  {
    id: 'v2',
    label: 'Team v2',
    summary: 'books only after approval, checks shelf life',
    rules: [
      'Book a shipment only after its cold_chain_shipment change is approved; never book first and ask after.',
      "Before allocating a lot, check its remaining shelf life on arrival against the customer's minimum; never ship a short-dated lot.",
      'Before booking, list the shipments already booked so no order is shipped twice.',
    ].join('\n'),
  },
  {
    id: 'checks-first',
    label: 'Team v2, cautious',
    summary: 'v2, and asks a person before any reallocation',
    rules: [
      'Book a shipment only after its cold_chain_shipment change is approved.',
      "Check each lot's remaining shelf life on arrival against the customer's minimum.",
      'Propose a stock_reallocation and wait for its decision before using reserved stock.',
    ].join('\n'),
  },
];

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
