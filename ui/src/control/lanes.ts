// The live action timeline (a sequence-of-events recorder): one lane per agent, each step a mark
// placed by its time. Tool calls are labelled by system kind and read/write; messages between
// agents name who they went to; a call to an approvals system, or one a gate refused, is an
// interlock.
import type { Event, Team } from '../types';
import { type Action, type Outcome, actionWord, classify, kindWord, worldInfo } from './actions';
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
  /** The world was saved after this step. */
  saved: boolean;
  /** An action: the kind of system it touched ("ERP"), the short label the timeline draws next to its shape. */
  system?: string;
  /** A message or handoff: the agent it went to (`*` for everyone, null when the record does not say). */
  to?: string | null;
}

export interface LaneRow {
  /** The agent id; `world` for the world's own steps (links, world events), `check` for scoring. */
  id: string;
  name: string;
  marks: Mark[];
}

export const WORLD_LANE = 'world';

/** Steps the world makes itself: its links (`link`) and a task's setup (`setup`), and events no agent made. */
export const isWorldActor = (agent: string | null) => agent == null || agent === 'link' || agent === 'setup';
export const CHECK_LANE = 'check';

const obj = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' ? (v as Record<string, unknown>) : {});

const APPROVALS = 'approvals';

function actionMark(a: Action): Mark {
  const interlock = a.system?.kind === APPROVALS || a.outcome === 'refused';
  return {
    seq: a.seq, at: a.at, kind: interlock ? 'interlock' : 'action',
    label: actionWord(a), detail: a.error ? `${a.tool}: ${a.error}` : a.tool,
    effect: a.effect, outcome: a.outcome, durationMs: a.durationMs, inherited: a.inherited, saved: a.saved,
    system: kindWord(a.system),
  };
}

type Base = Pick<Mark, 'seq' | 'at' | 'outcome' | 'durationMs' | 'inherited' | 'saved'>;
type Payload = Record<string, unknown>;

function messageMark(e: Event, p: Payload, base: Base): Mark {
  const raw = typeof p.to === 'string' ? p.to : null;
  const to = raw == null ? 'next' : raw === '*' ? 'everyone' : raw;
  const word = e.kind === 'handoff' ? 'Handoff' : 'Message';
  return { ...base, kind: 'message', label: `${word} → ${to}`, detail: String(p.text ?? p.note ?? ''), to: raw };
}

function endMark(p: Payload, base: Base): Mark {
  const failed = p.ok === false;
  return { ...base, kind: 'end', label: failed ? 'Ended: failed' : 'Ended', detail: String(p.error ?? ''), outcome: failed ? 'error' : 'ok' };
}

const MARKS: Partial<Record<Event['kind'], (e: Event, p: Payload, base: Base) => Mark>> = {
  message: messageMark,
  handoff: messageMark,
  model_call: (_e, _p, base) => ({ ...base, kind: 'model', label: 'Model call', detail: '' }),
  agent_start: (_e, _p, base) => ({ ...base, kind: 'start', label: 'Start', detail: '' }),
  agent_end: (_e, p, base) => endMark(p, base),
  check: (_e, p, base) => ({ ...base, kind: 'check', label: p.passed ? 'Check passed' : 'Check failed', detail: '', outcome: p.passed ? 'ok' : 'error' }),
  world_event: (_e, p, base) => ({ ...base, kind: 'world', label: `${String(p.part ?? '')} ${String(p.kind ?? 'event')}`.trim(), detail: String(p.text ?? '') }),
};

/** The mark of any step but a tool call. */
function stepMark(e: Event, inherited: boolean): Mark | null {
  const make = MARKS[e.kind];
  if (!make) return null;
  return make(e, obj(e.payload), { seq: e.seq, at: Date.parse(e.at), outcome: 'ok', durationMs: null, inherited, saved: false });
}

function laneOf(e: Event): string {
  if (e.kind === 'check') return CHECK_LANE;
  if (e.kind === 'world_event' || isWorldActor(e.agent)) return WORLD_LANE;
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

/** The time the marks cover, and `now` while live (at least one second, so a single mark has room). */
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
