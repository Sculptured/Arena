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

/** A time an agent of a dispatch run waited for work (`end` null: it was still waiting). */
export interface Gap {
  start: number;
  end: number | null;
}

export interface LaneRow {
  /** The agent id; `world` for the world's own steps (links, world events), `check` for scoring. */
  id: string;
  name: string;
  marks: Mark[];
  /** When the agent waited, idle, for work (dispatch runs only). */
  gaps?: Gap[];
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

/** A dispatch run's delegation (lead → agent) or report (agent → lead), drawn as a message. */
function dispatchMark(e: Event, p: Payload, base: Base): Mark {
  const to = typeof p.to === 'string' ? p.to : null;
  if (e.kind === 'delegation') return { ...base, kind: 'message', label: `Delegate → ${to ?? '?'}`, detail: String(p.task ?? ''), to };
  const word = p.turn_end ? 'Report (turn end)' : 'Report';
  return { ...base, kind: 'message', label: `${word} → ${to ?? 'lead'}`, detail: String(p.text ?? ''), to };
}

/** Why a dispatch run woke an agent, in a few words. */
export function wakeWord(p: Payload): string {
  switch (p.reason) {
    case 'start': return 'Started with the ticket';
    case 'idle': return `Woken: the team was waiting (${String(p.idle_wake ?? '')})`;
    case 'resume': return 'Carried on after the fork';
    default: {
      const n = Array.isArray(p.mail) ? p.mail.length : 0;
      return n > 1 ? `Woken by ${n} messages` : 'Woken by work or a message';
    }
  }
}

function endMark(p: Payload, base: Base): Mark {
  const failed = p.ok === false;
  return { ...base, kind: 'end', label: failed ? 'Ended: failed' : 'Ended', detail: String(p.error ?? ''), outcome: failed ? 'error' : 'ok' };
}

const MARKS: Partial<Record<Event['kind'], (e: Event, p: Payload, base: Base) => Mark>> = {
  message: messageMark,
  handoff: messageMark,
  delegation: dispatchMark,
  report: dispatchMark,
  wake: (_e, p, base) => ({ ...base, kind: 'start', label: wakeWord(p), detail: '' }),
  finish: (_e, p, base) => ({ ...base, kind: 'end', label: 'Finished the run', detail: String(p.summary ?? '') }),
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
  for (const [id, row] of rows) {
    const gaps = gapsOf(sorted, id);
    if (gaps.length) row.gaps = gaps;
  }
  const special = [WORLD_LANE, CHECK_LANE];
  const all = [...rows.values()];
  const agents = all.filter((r) => !special.includes(r.id));
  const tail = special.map((id) => rows.get(id)).filter((r): r is LaneRow => !!r && r.marks.length > 0);
  return [...agents, ...tail];
}

/**
 * When `agent` waited for work in a dispatch run: from its start (or a turn's end) to the next
 * wake, or to its end (null while it still waits). None for a run without wakes.
 */
export function gapsOf(sorted: Event[], agent: string): Gap[] {
  const mine = sorted.filter((e) => e.agent === agent && ['agent_start', 'wake', 'idle', 'agent_end'].includes(e.kind));
  if (!mine.some((e) => e.kind === 'wake')) return [];
  const gaps: Gap[] = [];
  let since: number | null = null;
  for (const e of mine) {
    const at = Date.parse(e.at);
    if (e.kind === 'agent_start' || e.kind === 'idle') since ??= at;
    else if (since != null) { gaps.push({ start: since, end: at }); since = null; }
  }
  if (since != null) gaps.push({ start: since, end: null });
  return gaps;
}

export interface Span {
  start: number;
  end: number;
}

/**
 * The time the marks cover (at least one second, so a single mark has room). A live run's span
 * grows only when a new event is recorded, never with the clock.
 */
export function spanOf(lanes: LaneRow[]): Span | null {
  const times = lanes.flatMap((l) => l.marks.map((m) => m.at)).filter((t) => !Number.isNaN(t));
  if (!times.length) return null;
  const start = Math.min(...times);
  const end = Math.max(...times);
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
