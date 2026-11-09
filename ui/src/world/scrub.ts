// The twin's scrubber: a track over the branch's steps, a stop at every view, marks where a
// disruption fired, a call was refused and the branch forked; what happened at a step; and where
// a rewind (the run's ⋯) forks.
import { isWorldActor } from '../control/lanes';
import { type Nav, moveIn } from '@/lib/keys';
import { actionsOf } from '../control/actions';
import type { Event, Team } from '../types';
import { isViewEvent } from './twin';
import type { BranchView } from './types';

export type TrackMarkKind = 'disruption' | 'refusal' | 'fork';

export interface TrackMark {
  seq: number;
  kind: TrackMarkKind;
  label: string;
}

export interface Track {
  min: number;
  max: number;
  /** The steps that have a view: where the scrubber stops. */
  stops: number[];
  marks: TrackMark[];
}

const obj = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' ? (v as Record<string, unknown>) : {});

/** A world event a part reported as a disruption (contract A: `{kind: "disruption", text, data}`). */
export function isDisruption(e: Event): boolean {
  return e.kind === 'world_event' && obj(e.payload).kind === 'disruption';
}

export const disruptionText = (e: Event) => String(obj(e.payload).text ?? 'disruption');

/** Disruptions and refused calls, in step order; the fork point of a forked branch. */
export function trackMarks(events: Event[], startSeq: number | null): TrackMark[] {
  const marks: TrackMark[] = events.filter(isDisruption).map((e) => ({ seq: e.seq, kind: 'disruption' as const, label: disruptionText(e) }));
  for (const a of actionsOf(events)) {
    if (a.outcome === 'refused') marks.push({ seq: a.seq, kind: 'refusal', label: `${a.tool} refused` });
  }
  if (startSeq != null) marks.push({ seq: startSeq, kind: 'fork', label: `forked here, at step ${startSeq}` });
  return marks.sort((a, b) => a.seq - b.seq);
}

/** The track over the branch's steps (views and world views aside) and its views' steps. */
export function buildTrack(events: Event[], views: BranchView[], startSeq: number | null): Track {
  const seqs = [...events.filter((e) => !isViewEvent(e) && e.kind !== 'run_start').map((e) => e.seq), ...views.map((v) => v.seq)];
  const min = seqs.length ? Math.min(...seqs) : 0;
  const max = seqs.length ? Math.max(...seqs) : 0;
  return { min, max, stops: views.map((v) => v.seq), marks: trackMarks(events, startSeq) };
}

/** Where a step falls on the track, from 0 to 1. */
export function trackAt(seq: number, t: Pick<Track, 'min' | 'max'>): number {
  if (t.max <= t.min) return 0;
  return Math.min(1, Math.max(0, (seq - t.min) / (t.max - t.min)));
}

/** The stop a step is shown at: the latest at or before it, else the first. */
export function snapStop(stops: number[], step: number | null): number | null {
  if (!stops.length) return null;
  if (step == null) return stops[stops.length - 1];
  return [...stops].reverse().find((s) => s <= step) ?? stops[0];
}

/** The stop after a key move from the stop `step` is shown at. */
export function moveStop(stops: number[], step: number | null, nav: Nav): number | null {
  return moveIn(stops, snapStop(stops, step), nav);
}

/** The step nearest a point on the track, among the stops. */
export function nearestStop(stops: number[], fraction: number, t: Pick<Track, 'min' | 'max'>): number | null {
  if (!stops.length) return null;
  const seq = t.min + fraction * (t.max - t.min);
  return stops.reduce((best, s) => (Math.abs(s - seq) < Math.abs(best - seq) ? s : best), stops[0]);
}

export interface Happening {
  seq: number;
  kind: 'disruption' | 'refusal' | 'error' | 'write';
  text: string;
}

/** What happened after step `after` up to step `upTo`: disruptions, refused or failed calls, writes. */
export function happenings(events: Event[], team: Team, after: number, upTo: number): Happening[] {
  const name = (id: string) => team.agents.find((a) => a.id === id)?.name ?? id;
  const inRange = (s: number) => s > after && s <= upTo;
  const out: Happening[] = events.filter((e) => inRange(e.seq) && isDisruption(e)).map((e) => ({ seq: e.seq, kind: 'disruption' as const, text: disruptionText(e) }));
  for (const a of actionsOf(events)) {
    if (!inRange(a.seq)) continue;
    const who = a.agent === 'link' || a.agent === 'world' ? 'World' : name(a.agent);
    if (a.outcome !== 'ok') out.push({ seq: a.seq, kind: a.outcome === 'refused' ? 'refusal' : 'error', text: `${who}: ${failureText(a.tool, a.outcome, a.error)}` });
    else if (a.effect === 'write') out.push({ seq: a.seq, kind: 'write', text: `${who}: ${a.tool}` });
  }
  return out.sort((a, b) => a.seq - b.seq);
}

/** "book_shipment refused: no approved change", without saying the tool twice when the error does. */
function failureText(tool: string, outcome: string, error: string | null): string {
  if (error?.startsWith(tool)) return error;
  return `${tool} ${outcome}${error ? `: ${error}` : ''}`;
}

const FORKABLE = new Set<Event['kind']>(['tool_call', 'model_call', 'message', 'handoff', 'agent_start', 'agent_end', 'delegation', 'report', 'wake', 'finish']);

/**
 * Where a rewind forks to keep the world exactly as it is after step `seq`: the first agent
 * step after it (a fork reruns from its step and reuses everything before). Null at the end.
 */
export function rewindSeq(events: Event[], seq: number): number | null {
  const next = events.filter((e) => e.seq > seq && FORKABLE.has(e.kind) && !isWorldActor(e.agent)).map((e) => e.seq);
  return next.length ? Math.min(...next) : null;
}
