// The live action timeline (a sequence-of-events recorder): one lane per agent, each step a mark
// placed by its time. Tool calls are labelled by system kind and read/write; messages between
// agents name who they went to; a call to an approvals system, or one a gate refused, is an
// interlock.
import type { Event, Team } from '../types';
import { type Action, type Outcome, actionWord, classify, worldInfo } from './actions';
import type { Effect } from './types';

export type MarkKind = 'action' | 'interlock' | 'message' | 'model' | 'start' | 'end' | 'check' | 'world';

export interface Mark {
  seq: number;
  at: number;
  kind: MarkKind;
  /** What the mark says: "ERP write", "message → quality", "model call". */
  label: string;
  /** The tool, the note, or the event's text: shown on hover. */
  detail: string;
  effect?: Effect | 'unknown';
  outcome: Outcome;
  durationMs: number | null;
  inherited: boolean;
}

export interface LaneRow {
  /** The agent id; `world` for the world's own steps (links, world events), `check` for scoring. */
  id: string;
  name: string;
  marks: Mark[];
}

export const WORLD_LANE = 'world';
export const CHECK_LANE = 'check';

const obj = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' ? (v as Record<string, unknown>) : {});

const APPROVALS = 'approvals';

function actionMark(a: Action): Mark {
  const interlock = a.system?.kind === APPROVALS || a.outcome === 'refused';
  return {
    seq: a.seq, at: a.at, kind: interlock ? 'interlock' : 'action',
    label: actionWord(a), detail: a.error ? `${a.tool}: ${a.error}` : a.tool,
    effect: a.effect, outcome: a.outcome, durationMs: a.durationMs, inherited: a.inherited,
  };
}

/** The mark of any step but a tool call. */
function stepMark(e: Event, inherited: boolean): Mark | null {
  const p = obj(e.payload);
  const base = { seq: e.seq, at: Date.parse(e.at), outcome: 'ok' as Outcome, durationMs: null, inherited };
  switch (e.kind) {
    case 'message':
    case 'handoff': {
      const to = typeof p.to === 'string' ? (p.to === '*' ? 'everyone' : p.to) : 'next';
      return { ...base, kind: 'message', label: `${e.kind} → ${to}`, detail: String(p.text ?? p.note ?? '') };
    }
    case 'model_call': return { ...base, kind: 'model', label: 'model call', detail: '' };
    case 'agent_start': return { ...base, kind: 'start', label: 'start', detail: '' };
    case 'agent_end': {
      const failed = p.ok === false;
      return { ...base, kind: 'end', label: failed ? 'ended: failed' : 'ended', detail: String(p.error ?? ''), outcome: failed ? 'error' : 'ok' };
    }
    case 'check': return { ...base, kind: 'check', label: p.passed ? 'check passed' : 'check failed', detail: '', outcome: p.passed ? 'ok' : 'error' };
    case 'world_event': return { ...base, kind: 'world', label: `${String(p.part ?? '')} ${String(p.kind ?? 'event')}`.trim(), detail: String(p.text ?? '') };
    default: return null;
  }
}

function laneOf(e: Event): string {
  if (e.kind === 'check') return CHECK_LANE;
  if (e.kind === 'world_event' || e.agent == null || e.agent === 'link') return WORLD_LANE;
  return e.agent;
}

/** Lanes in team order, then any agent that joined, then the world and the check (when they have marks). */
export function buildLanes(events: Event[], team: Team, startSeq: number | null = null): LaneRow[] {
  const info = worldInfo(events);
  const rows = new Map<string, LaneRow>(team.agents.map((a) => [a.id, { id: a.id, name: a.name, marks: [] }]));
  const sorted = [...events].sort((a, b) => a.seq - b.seq);
  for (const e of sorted) {
    if (e.kind === 'run_start') continue;
    const inherited = startSeq != null && e.seq < startSeq;
    const action = classify(e, info, startSeq);
    const mark = action ? actionMark(action) : stepMark(e, inherited);
    if (!mark) continue;
    const id = laneOf(e);
    if (!rows.has(id)) rows.set(id, { id, name: id === WORLD_LANE ? 'World' : id === CHECK_LANE ? 'Check' : id, marks: [] });
    rows.get(id)!.marks.push(mark);
  }
  const special = [WORLD_LANE, CHECK_LANE];
  const all = [...rows.values()];
  const agents = all.filter((r) => !special.includes(r.id));
  const tail = special.map((id) => rows.get(id)).filter((r): r is LaneRow => !!r && r.marks.length > 0);
  return [...agents, ...tail];
}

export interface Span {
  start: number;
  end: number;
}

/** The time the marks cover (at least one second, so a single mark has room). */
export function spanOf(lanes: LaneRow[], now?: number): Span | null {
  const times = lanes.flatMap((l) => l.marks.map((m) => m.at)).filter((t) => !Number.isNaN(t));
  if (!times.length) return null;
  const start = Math.min(...times);
  const end = Math.max(...times, now ?? -Infinity);
  return { start, end: Math.max(end, start + 1000) };
}

/** Where a time falls in a span, from 0 to 1. */
export function position(at: number, span: Span): number {
  return Math.min(1, Math.max(0, (at - span.start) / (span.end - span.start)));
}

/** "1.2 s", "340 ms", "–". */
export function formatDuration(ms: number | null): string {
  if (ms == null) return '–';
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)} s` : `${Math.round(ms)} ms`;
}
