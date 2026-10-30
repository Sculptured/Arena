import type { Event, Team } from '../types';

export type StepKind = 'start' | 'model' | 'tool' | 'handoff' | 'end' | 'check';

export interface Step {
  seq: number;
  kind: StepKind;
  label: string;
  wrote: boolean;
  snapshot: boolean;
  inherited: boolean;
}

export interface Lane {
  agent: string;
  steps: Step[];
}

export const CHECK_LANE = 'check';

const nonEmpty = (v: unknown) => Array.isArray(v) && v.length > 0;

/** One lane per agent in chain order, then a lane for the check. Steps below `startSeq` are inherited. */
export function lanes(events: Event[], team: Team, startSeq: number | null = null): Lane[] {
  const byAgent = new Map<string, Step[]>(team.agents.map((a) => [a.id, []]));
  const checks: Step[] = [];
  for (const e of [...events].sort((a, b) => a.seq - b.seq)) {
    if (e.kind === 'run_start') continue;
    const p = (e.payload ?? {}) as Record<string, unknown>;
    const step: Step = {
      seq: e.seq, kind: 'start', label: '', wrote: false, snapshot: false,
      inherited: startSeq != null && e.seq < startSeq,
    };
    switch (e.kind) {
      case 'agent_start': step.kind = 'start'; step.label = 'start'; break;
      case 'agent_end': step.kind = 'end'; step.label = p.ok === false ? 'end (failed)' : 'end'; break;
      case 'model_call': step.kind = 'model'; step.label = 'model'; break;
      case 'handoff': step.kind = 'handoff'; step.label = 'handoff'; break;
      case 'tool_call':
        step.kind = 'tool';
        step.label = typeof p.name === 'string' ? p.name : 'tool';
        step.wrote = nonEmpty(p.writes);
        step.snapshot = typeof p.snapshot === 'string' && p.snapshot !== '';
        break;
      case 'check': step.kind = 'check'; step.label = p.passed ? 'check passed' : 'check failed'; break;
    }
    if (e.kind === 'check' || e.agent == null) {
      checks.push(step);
      continue;
    }
    if (!byAgent.has(e.agent)) byAgent.set(e.agent, []);
    byAgent.get(e.agent)!.push(step);
  }
  const out: Lane[] = [...byAgent].map(([agent, steps]) => ({ agent, steps }));
  if (checks.length) out.push({ agent: CHECK_LANE, steps: checks });
  return out;
}

export function costLabel(source: string | null | undefined): string {
  if (source === 'api_equivalent_estimate') return 'API-equivalent estimate';
  if (source === 'openrouter') return 'OpenRouter';
  return '';
}

export function formatCost(usd: number | null | undefined, source: string | null | undefined): string {
  if (usd == null) return 'tokens only';
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
