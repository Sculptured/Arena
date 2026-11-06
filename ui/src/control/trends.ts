// Trends: actions per minute per system, the context each model call was sent with, and tokens
// and cost as they build up.
import type { Event } from '../types';
import { type Action, kindWord } from './actions';

export interface Series {
  name: string;
  values: number[];
}

export interface Bucketed {
  /** Each bucket's start, ms since the epoch. */
  starts: number[];
  bucketMs: number;
  series: Series[];
}

/** Actions per bucket (a minute by default), one series per system kind, in order of first use. */
export function actionsPerBucket(actions: Action[], bucketMs = 60_000): Bucketed {
  const timed = actions.filter((a) => !Number.isNaN(a.at));
  if (!timed.length) return { starts: [], bucketMs, series: [] };
  const first = Math.min(...timed.map((a) => a.at));
  const last = Math.max(...timed.map((a) => a.at));
  const n = Math.floor((last - first) / bucketMs) + 1;
  const series = new Map<string, number[]>();
  for (const a of timed) {
    const k = kindWord(a.system);
    if (!series.has(k)) series.set(k, new Array(n).fill(0));
    series.get(k)![Math.floor((a.at - first) / bucketMs)] += 1;
  }
  return {
    starts: Array.from({ length: n }, (_, i) => first + i * bucketMs),
    bucketMs,
    series: [...series].map(([name, values]) => ({ name, values })),
  };
}

export interface CallPoint {
  seq: number;
  at: number;
  agent: string;
  /** The context the call was sent with (`usage.prompt_tokens`). */
  context: number;
  tokens: number;
  costUsd: number | null;
}

const obj = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' ? (v as Record<string, unknown>) : {});
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);

/** Every model call with a known context size (compactions and calls without usage are left out). */
export function modelCalls(events: Event[]): CallPoint[] {
  const out: CallPoint[] = [];
  for (const e of events) {
    if (e.kind !== 'model_call') continue;
    const p = obj(e.payload);
    const usage = obj(p.usage);
    const context = num(usage.prompt_tokens);
    if (context == null) continue;
    const tokens = num(usage.total_tokens) ?? context + (num(usage.completion_tokens) ?? 0);
    out.push({ seq: e.seq, at: Date.parse(e.at), agent: e.agent ?? '', context, tokens, costUsd: num(p.cost_usd) });
  }
  return out;
}

export interface Cumulative {
  tokens: number[];
  /** Null from the first call whose cost is unknown on: a total is unknown once any part is. */
  cost: (number | null)[];
}

export function cumulative(calls: CallPoint[]): Cumulative {
  let tokens = 0;
  let cost: number | null = 0;
  const out: Cumulative = { tokens: [], cost: [] };
  for (const c of calls) {
    tokens += c.tokens;
    cost = cost != null && c.costUsd != null ? cost + c.costUsd : null;
    out.tokens.push(tokens);
    out.cost.push(cost);
  }
  return out;
}

/** The points of a sparkline of `values` in a `w` × `h` box (SVG `points`). */
export function sparkPoints(values: number[], w: number, h: number): string {
  if (!values.length) return '';
  const max = Math.max(...values, 1);
  const step = values.length > 1 ? w / (values.length - 1) : 0;
  return values.map((v, i) => `${(i * step).toFixed(1)},${(h - (v / max) * h).toFixed(1)}`).join(' ');
}
