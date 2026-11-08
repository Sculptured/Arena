// Where the action timeline draws things: each mark at its time in its agent's lane, the
// connectors of messages, handoffs, delegations and reports between lanes, each lane's active span
// (with the times a dispatch run's agent waited for work as faint gaps), the time ticks,
// and which marks have room for their label (system kind and read/write).
import { type LaneRow, type Mark, type Span, position } from '../control/lanes';
import type { Pause } from './flow';

export const LANE_H = 36;
/** Space kept clear at each end of the track so edge marks are whole. */
export const TRACK_PAD = 14;

export type Shape =
  | 'read' | 'write' | 'call' | 'gate-read' | 'gate-write' | 'abnormal'
  | 'message' | 'model' | 'start' | 'end' | 'end-failed' | 'check-pass' | 'check-fail' | 'world';

/** Reads hollow, writes solid (heavier), approvals a diamond; refusals and errors a triangle in the alarm colour. */
export function shapeOf(m: Mark): Shape {
  const ok = m.outcome === 'ok';
  switch (m.kind) {
    case 'action': return !ok ? 'abnormal' : m.effect === 'write' ? 'write' : m.effect === 'read' ? 'read' : 'call';
    case 'interlock': return !ok ? 'abnormal' : m.effect === 'write' ? 'gate-write' : 'gate-read';
    case 'end': return ok ? 'end' : 'end-failed';
    case 'check': return ok ? 'check-pass' : 'check-fail';
    default: return m.kind;
  }
}

/** The text a mark carries when there is room: the system kind of an action ("ERP"; its shape says read or write), "→ quality", "check passed". */
export function shortLabel(m: Mark): string {
  if (m.system && (m.kind === 'action' || m.kind === 'interlock')) return m.system;
  if (m.kind === 'message') return `→ ${m.to === '*' ? 'everyone' : m.to ?? 'next'}`;
  if (m.kind === 'model' || m.kind === 'start') return '';
  if (m.kind === 'end') return m.outcome === 'ok' ? '' : 'Failed';
  return m.label;
}

export interface Placed {
  mark: Mark;
  lane: number;
  x: number;
  /** The label fits before the next mark in the lane. */
  labelled: boolean;
}

export interface Connector {
  seq: number;
  x: number;
  from: number;
  to: number;
}

export interface Tick {
  x: number;
  label: string;
}

export interface Bar {
  lane: number;
  x1: number;
  x2: number;
}

/** A time an agent waited for work, drawn faint on its lane. */
export interface GapBar {
  lane: number;
  x1: number;
  x2: number;
}

/** A pause drawn across the lanes: nothing could happen in it. */
export interface Band {
  x1: number;
  x2: number;
  open: boolean;
}

export interface Layout {
  placed: Placed[];
  connectors: Connector[];
  ticks: Tick[];
  bars: Bar[];
  bands: Band[];
  gaps: GapBar[];
}

/** How far right of its mark's centre a label reaches at 11 px (shape, gap, text, padding), plus room before the next shape. */
const labelReach = (text: string) => 10 + text.length * 6.8 + 4 + 7;

/** Marks in x order; each labelled when no other mark (model ticks aside) sits under its label. */
function placeLane(marks: Mark[], lane: number, xOf: (at: number) => number, width: number): Placed[] {
  const placed = marks.map((mark) => ({ mark, lane, x: xOf(mark.at), labelled: false })).sort((a, b) => a.x - b.x);
  const solid = placed.filter((p) => p.mark.kind !== 'model');
  let clearFrom = -Infinity;
  solid.forEach((p, i) => {
    const text = shortLabel(p.mark);
    if (!text || p.x < clearFrom) return;
    const end = p.x + labelReach(text);
    const next = solid[i + 1];
    if ((next && next.x < end) || end > width) return;
    p.labelled = true;
    clearFrom = end;
  });
  return placed;
}

const STEPS_S = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600, 7200];

/** A tick every "nice" number of seconds, about one per `every` pixels. */
export function ticksOf(span: Span, width: number, every = 96): Tick[] {
  const secs = (span.end - span.start) / 1000;
  const want = Math.max(1, Math.floor((width - 2 * TRACK_PAD) / every));
  const step = STEPS_S.find((s) => secs / s <= want) ?? STEPS_S[STEPS_S.length - 1];
  const out: Tick[] = [];
  for (let s = 0; s <= secs + 1e-9; s += step) {
    out.push({ x: TRACK_PAD + (s / secs) * (width - 2 * TRACK_PAD), label: s >= 60 && step >= 60 ? `${s / 60} min` : `${s} s` });
  }
  return out;
}

function connectorsOf(placed: Placed[], laneIndex: Map<string, number>): Connector[] {
  return placed.flatMap((p) => {
    const to = p.mark.kind === 'message' && p.mark.to ? laneIndex.get(p.mark.to) : undefined;
    return to == null || to === p.lane ? [] : [{ seq: p.mark.seq, x: p.x, from: p.lane, to }];
  });
}

/** The whole layout for a track `width` pixels wide; `models` false leaves model calls out; `pauses` become bands. */
export function layoutTimeline(lanes: LaneRow[], span: Span, width: number, models = true, pauses: Pause[] = []): Layout {
  const xOf = (at: number) => (Number.isNaN(at) ? TRACK_PAD : TRACK_PAD + position(at, span) * (width - 2 * TRACK_PAD));
  const laneIndex = new Map(lanes.map((l, i) => [l.id, i]));
  const placed = lanes.flatMap((l, i) => placeLane(l.marks.filter((m) => models || m.kind !== 'model'), i, xOf, width));
  const bars = lanes.flatMap((_, i) => {
    const xs = placed.filter((p) => p.lane === i).map((p) => p.x);
    return xs.length > 1 ? [{ lane: i, x1: Math.min(...xs), x2: Math.max(...xs) }] : [];
  });
  const bands = pauses.map((p) => ({ x1: xOf(p.start), x2: Math.max(xOf(p.end), xOf(p.start) + 2), open: p.open }));
  const gaps = lanes.flatMap((l, i) => (l.gaps ?? []).map((g) => ({ lane: i, x1: xOf(g.start), x2: xOf(g.end ?? span.end) })))
    .filter((g) => g.x2 - g.x1 >= 1);
  return { placed, connectors: connectorsOf(placed, laneIndex), ticks: ticksOf(span, width), bars, bands, gaps };
}

/** Every step in the order the timeline moves through with ←/→ (model calls only when shown). */
export function stepOrder(lanes: LaneRow[], models = true): number[] {
  return lanes.flatMap((l) => l.marks).filter((m) => models || m.kind !== 'model').map((m) => m.seq).sort((a, b) => a - b);
}
