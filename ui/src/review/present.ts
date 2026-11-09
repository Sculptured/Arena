import type { StoredReview } from './types';

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

/** The run page at a step. */
export const stepHref = (runId: string, branchId: string, seq: number) =>
  `/runs/${encodeURIComponent(runId)}/${encodeURIComponent(branchId)}?step=${seq}`;

export const SEVERITY_LABEL = { safety: 'Safety', goal: 'Goal', efficiency: 'Efficiency' } as const;

export const KIND_LABEL = { prompt: 'Prompt', team: 'Team', tool: 'Tool', model: 'Model', process: 'Process' } as const;
