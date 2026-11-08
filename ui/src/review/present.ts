import type { Change, Event, Team } from '../types';
import { rewindSeq } from '../world/scrub';
import type { StoredReview, Suggestion } from './types';

/** The reviewer as a person reads it: "Claude Code sonnet", "openrouter deepseek/…". */
export function modelName(label: string): string {
  const [kind, ...rest] = label.split(':');
  const model = rest.join(':');
  return kind === 'claude_code' ? `Claude Code ${model}` : `${kind} ${model}`.trim();
}

/** The small print under a review: who reviewed, what it cost, how long it took. */
export function smallPrint(s: StoredReview): string {
  const parts = [`Reviewed by ${modelName(s.model)}`];
  if (s.cost_usd != null) parts.push(`$${s.cost_usd.toFixed(s.cost_usd < 0.1 ? 3 : 2)}${s.cost_source === 'api_equivalent_estimate' ? ' (API-equivalent)' : ''}`);
  else if (s.tokens != null) parts.push(`${s.tokens.toLocaleString()} tokens`);
  if (s.seconds != null) parts.push(`${Math.round(s.seconds)} s`);
  return parts.join(' · ');
}

/** A suggestion that "Rewind and try this" can pre-fill: a prompt or team change with text to add. */
export const canTry = (s: Suggestion) => (s.kind === 'prompt' || s.kind === 'team') && !!s.prompt_text?.trim();

/**
 * The fork changes a suggestion pre-fills: its text added to its agent's prompt, or to every
 * agent's when it names none. Null when it has nothing to pre-fill.
 */
export function prefillOf(s: Suggestion, team: Team): Record<string, Change> | null {
  if (!canTry(s)) return null;
  const text = s.prompt_text!.trim();
  const agents = s.agent ? team.agents.filter((a) => a.id === s.agent) : team.agents;
  if (!agents.length) return null;
  return Object.fromEntries(agents.map((a) => [a.id, { prompt: `${a.prompt.trimEnd()}\n\n${text}` }]));
}

/**
 * The step to rerun from to try a suggestion: its own step, else its agent's start, else the
 * run's first agent step; moved to the first step at or after it that a rerun can start from.
 */
export function tryStep(s: Suggestion, events: Event[]): number | null {
  const agentStart = s.agent ? events.find((e) => e.kind === 'agent_start' && e.agent === s.agent)?.seq : undefined;
  const at = s.step ?? agentStart ?? 0;
  return rewindSeq(events, at - 1);
}

/** The run page at a step. */
export const stepHref = (runId: string, branchId: string, seq: number) =>
  `/runs/${encodeURIComponent(runId)}/${encodeURIComponent(branchId)}?step=${seq}`;

export const SEVERITY_LABEL = { safety: 'Safety', goal: 'Goal', efficiency: 'Efficiency' } as const;

export const KIND_LABEL = { prompt: 'Prompt', team: 'Team', tool: 'Tool', model: 'Model', process: 'Process' } as const;
