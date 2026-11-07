// The divergence strip: both branches on one step axis, where they first diverge, where each
// one's disruptions fired and its calls were refused.
import type { Event } from '../types';
import { type TrackMark, trackMarks } from '../world/scrub';
import { isViewEvent } from '../world/twin';

export interface StripRow {
  side: 'A' | 'B';
  /** The last step of the branch. */
  end: number;
  marks: TrackMark[];
}

export interface Strip {
  min: number;
  max: number;
  first: number | null;
  rows: StripRow[];
  /** Step numbers to label on the axis. */
  ticks: number[];
}

const steps = (events: Event[]) => events.filter((e) => e.kind !== 'run_start' && !isViewEvent(e)).map((e) => e.seq);

/** Round tick steps (1, 2, 5, 10, ...) for about `want` labels from `min` to `max`. */
export function axisTicks(min: number, max: number, want = 6): number[] {
  if (max <= min) return [min];
  const raw = (max - min) / want;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 5, 10].map((m) => m * mag).find((s) => s >= raw) ?? 10 * mag;
  const out: number[] = [];
  for (let t = Math.ceil(min / step) * step; t <= max; t += step) out.push(t);
  return out;
}

/** The strip for branches A and B; a fork's own fork point is one of its marks. */
export function divergenceStrip(a: { events: Event[]; start_seq: number | null }, b: { events: Event[]; start_seq: number | null }, first: number | null): Strip {
  const all = [...steps(a.events), ...steps(b.events)];
  const min = all.length ? Math.min(...all) : 0;
  const max = all.length ? Math.max(...all) : 0;
  const row = (side: 'A' | 'B', x: typeof a): StripRow => ({ side, end: Math.max(min, ...steps(x.events)), marks: trackMarks(x.events, x.start_seq) });
  return { min, max, first, rows: [row('A', a), row('B', b)], ticks: axisTicks(min, max) };
}
