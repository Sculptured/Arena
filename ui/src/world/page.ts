// The world view's words: who drove the agents, and what the step on screen was.
import { isWorldActor } from '../control/lanes';
import type { Event, Team } from '../types';
import { type Happening, type Track, buildTrack, happenings } from './scrub';
import { type ViewChanges, diffViews, viewIndexAt } from './twin';
import type { BranchView, TwinView } from './types';

const obj = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' ? (v as Record<string, unknown>) : {});

/** An agent's end that reports tokens: a CLI agent's model calls aren't recorded one by one, but they happened. */
const reportsTokens = (e: Event) => e.kind === 'agent_end' && Number(obj(e.payload).tokens ?? 0) > 0;

/** "No model calls" for a run that recorded none and whose agents report no tokens (nothing billed); else empty. */
export function drivenLine(events: Event[]): string {
  return events.some((e) => e.kind === 'model_call' || reportsTokens(e)) ? '' : 'No model calls';
}

/** "Step 21 · Logistics · book_shipment": the step a view shows. */
export function stepHeadline(events: Event[], team: Team, seq: number | null): string {
  const e = events.find((x) => x.seq === seq);
  if (seq == null || !e) return 'No step';
  const name = (id: string | null) => (isWorldActor(id) ? 'World' : team.agents.find((a) => a.id === id)?.name ?? id);
  const p = obj(e.payload);
  if (e.kind === 'run_start') return `Step ${seq} · after setup`;
  if (e.kind === 'tool_call') return `Step ${seq} · ${name(e.agent)} · ${String(p.name ?? 'tool')}`;
  if (e.kind === 'world_event') return `Step ${seq} · World · ${String(p.kind ?? 'event')}`;
  return `Step ${seq} · ${name(e.agent)} · ${e.kind.replace('_', ' ')}`;
}

export interface WorldInput {
  views: BranchView[];
  step: number | null;
  events: Event[];
  team: Team;
  startSeq: number | null;
}

export interface WorldState {
  index: number;
  /** The step of the view on screen. */
  current: number | null;
  view: TwinView | null;
  changes: ViewChanges;
  track: Track;
  happened: Happening[];
  headline: string;
}

/** Everything the world view shows for the step in the address (the last view when none). */
export function worldState(i: WorldInput): WorldState {
  const index = viewIndexAt(i.views, i.step);
  const view: BranchView | null = i.views[index] ?? null;
  const prev: BranchView | null = i.views[index - 1] ?? null;
  const current = view ? view.seq : null;
  return {
    index, current, view: view ? view.view : null,
    changes: diffViews(prev && prev.view, view && view.view),
    track: buildTrack(i.events, i.views, i.startSeq),
    happened: view ? happenings(i.events, i.team, prev ? prev.seq : 0, view.seq) : [],
    headline: stepHeadline(i.events, i.team, current),
  };
}
