// The trace waterfall of one turn: an agent's model call and every step it led to, up to its
// next model call. Time runs left to right; a step's bar ends when it was recorded and starts
// that long before when its duration is known; arena's own work in a step (gate checks,
// recording, saves) is listed under it. Gaps are waiting or unseen work.
import { type WorldInfo, actionWord, classify, worldInfo } from '../control/actions';
import type { Event } from '../types';

export type SpanKind = 'turn' | 'step' | 'overhead';

export interface TraceSpan {
  /** The step a row selects; null for arena overhead rows. */
  seq: number | null;
  kind: SpanKind;
  label: string;
  detail: string;
  /** Milliseconds from the turn's start. */
  start: number;
  end: number;
  depth: number;
  abnormal: boolean;
  write: boolean;
}

export interface Trace {
  agent: string;
  /** "turn 3 of 12" in the agent's own count. */
  title: string;
  spans: TraceSpan[];
  /** Milliseconds the turn covers (at least 1). */
  total: number;
}

const OVERHEAD_WORDS: Record<string, string> = {
  gate: 'gate check', record: 'recording', wait: 'waiting for the step lock', save: 'save', link: 'link call',
  mcp: 'MCP endpoint', restore: 'restore', gateway: 'gateway hop', score: 'scoring',
};

const obj = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' ? (v as Record<string, unknown>) : {});

/** The agent's steps cut into turns: each model call starts one; steps before the first form turn 0. */
export function turnsOf(events: Event[], agent: string): Event[][] {
  const mine = events.filter((e) => e.agent === agent && e.kind !== 'run_start').sort((a, b) => a.seq - b.seq);
  const turns: Event[][] = [];
  for (const e of mine) {
    if (e.kind === 'model_call' || turns.length === 0) turns.push([]);
    turns[turns.length - 1].push(e);
  }
  return turns;
}

function stepLabel(e: Event, info: WorldInfo): { label: string; detail: string; abnormal: boolean; write: boolean } {
  const a = classify(e, info);
  if (a) return { label: `${actionWord(a)} · ${a.tool}`, detail: a.error ?? '', abnormal: a.outcome !== 'ok', write: a.effect === 'write' };
  const p = obj(e.payload);
  if (e.kind === 'model_call') return { label: 'Model call', detail: typeof p.model === 'string' ? p.model : '', abnormal: false, write: false };
  if (['message', 'handoff', 'delegation', 'report'].includes(e.kind)) return { label: `${e.kind} → ${String(p.to ?? 'next')}`, detail: '', abnormal: false, write: false };
  if (e.kind === 'agent_end') return { label: p.ok === false ? 'Ended: failed' : 'Ended', detail: String(p.error ?? ''), abnormal: p.ok === false, write: false };
  return { label: e.kind.replace('_', ' '), detail: '', abnormal: false, write: false };
}

const durationOf = (e: Event) => {
  const d = obj(e.payload).duration_ms;
  return typeof d === 'number' ? d : 0;
};

/** "turn 3 of 12" counting model calls; "before its first model call" for steps ahead of any. */
function turnTitle(turns: Event[][], index: number): string {
  const pre = turns[0][0].kind !== 'model_call';
  if (pre && index === 0) return 'before its first model call';
  return `turn ${index + (pre ? 0 : 1)} of ${turns.length - (pre ? 1 : 0)}`;
}

/** A step's bar and, under it, one row per kind of arena overhead it recorded. */
function stepSpans(e: Event, t0: number, info: WorldInfo): TraceSpan[] {
  const p = obj(e.payload);
  const end = Date.parse(e.at) - t0;
  const dur = durationOf(e);
  const step: TraceSpan = { seq: e.seq, kind: 'step', ...stepLabel(e, info), start: end - dur, end, depth: 1 };
  const over = Object.entries(obj(p.overhead_ms)).filter(([, v]) => typeof v === 'number' && v > 0.05) as [string, number][];
  const rows = over.map(([k, ms]): TraceSpan => ({
    seq: null, kind: 'overhead', label: `arena · ${OVERHEAD_WORDS[k] ?? k}`, detail: '', start: step.start, end: step.start + ms, depth: 2, abnormal: false, write: false,
  }));
  return [step, ...rows];
}

/** The trace of the turn holding step `seq`; null for a step outside any agent (the check, the world). */
export function traceOf(events: Event[], seq: number): Trace | null {
  const ev = events.find((e) => e.seq === seq);
  if (!ev?.agent || ev.kind === 'run_start') return null;
  const turns = turnsOf(events, ev.agent);
  const index = turns.findIndex((t) => t.some((e) => e.seq === seq));
  const turn = turns[index];
  const info = worldInfo(events);
  const t0 = Math.min(...turn.map((e) => Date.parse(e.at) - durationOf(e)));
  const steps = turn.flatMap((e) => stepSpans(e, t0, info));
  const total = Math.max(1, ...steps.map((s) => s.end));
  const title = turnTitle(turns, index);
  const root: TraceSpan = { seq: null, kind: 'turn', label: `${ev.agent} · ${title}`, detail: '', start: 0, end: total, depth: 0, abnormal: steps.some((s) => s.abnormal), write: false };
  return { agent: ev.agent, title, spans: [root, ...steps], total };
}

/** "120 ms", "1.4 s", "0.03 ms": a span's length. */
export function spanLength(ms: number): string {
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)} s`;
  if (ms >= 10) return `${Math.round(ms)} ms`;
  return `${ms.toFixed(ms >= 1 ? 1 : 2)} ms`;
}
