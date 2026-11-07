// Where a run stands in plain words (Starting, Running, Paused, Passed, Failed, Stopped), and the
// pauses it had, from its record: `pause` and `resume` events, and its status.
import type { Mark } from '@/lib/mark';
import type { BranchDetail, Event } from '../types';

export type Flow = 'starting' | 'running' | 'paused' | 'passed' | 'failed' | 'stopped' | 'error' | 'interrupted';

const MARK: Record<Flow, Mark> = {
  starting: { glyph: '◔', word: 'Starting', abnormal: false },
  running: { glyph: '◐', word: 'Running', abnormal: false },
  paused: { glyph: '‖', word: 'Paused', abnormal: false },
  passed: { glyph: '✓', word: 'Passed', abnormal: false },
  failed: { glyph: '✕', word: 'Failed', abnormal: true },
  stopped: { glyph: '■', word: 'Stopped', abnormal: false },
  error: { glyph: '⚠', word: 'Error', abnormal: true },
  interrupted: { glyph: '■', word: 'Interrupted', abnormal: true },
};

export const flowMark = (f: Flow): Mark => MARK[f];

/** True when the last of the branch's `pause` and `resume` events is a pause. */
export function pausedIn(events: Event[]): boolean {
  const last = [...events].sort((a, b) => b.seq - a.seq).find((e) => e.kind === 'pause' || e.kind === 'resume');
  return last?.kind === 'pause';
}

/** The run's flow (paused as its record says); `null` detail while it starts (its world is being restored). */
export function flowOf(d: Pick<BranchDetail, 'status' | 'events'> | null): Flow {
  if (!d) return 'starting';
  switch (d.status) {
    case 'running': return pausedIn(d.events) ? 'paused' : 'running';
    case 'infra_error': return 'error';
    default: return d.status;
  }
}

export const isLive = (f: Flow) => f === 'starting' || f === 'running' || f === 'paused';

export interface Pause {
  start: number;
  /** When it ended; `now` for a pause still open. */
  end: number;
  open: boolean;
}

/** Each pause's span in ms since the epoch, in order; one still open ends at `now`. */
export function pausesOf(events: Event[], now: number = Date.now()): Pause[] {
  const out: Pause[] = [];
  let start: number | null = null;
  for (const e of [...events].sort((a, b) => a.seq - b.seq)) {
    if (e.kind === 'pause' && start == null) start = Date.parse(e.at);
    if (e.kind === 'resume' && start != null) {
      out.push({ start, end: Date.parse(e.at), open: false });
      start = null;
    }
  }
  if (start != null) out.push({ start, end: Math.max(start, now), open: true });
  return out;
}

/** "42 s", "3 min 05 s", "1 h 02 min": an elapsed time, calm and short. */
export function clockText(seconds: number | null): string {
  if (seconds == null || !Number.isFinite(seconds)) return '–';
  const s = Math.max(0, Math.floor(seconds));
  if (s < 60) return `${s} s`;
  const pad = (n: number) => String(n).padStart(2, '0');
  if (s < 3600) return `${Math.floor(s / 60)} min ${pad(s % 60)} s`;
  return `${Math.floor(s / 3600)} h ${pad(Math.floor((s % 3600) / 60))} min`;
}
