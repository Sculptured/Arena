import { describe, expect, it } from 'vitest';
import { layoutTimeline } from '../run/timelineLayout';
import { buildLanes, gapsOf, spanOf } from './lanes';
import { ev, runStart, team } from './fixtures';

/** A dispatch run: the lead delegates to quality, which works, reports, and waits; the lead finishes. */
const dispatched = [
  runStart,
  ev(2, 1, 'lead', 'agent_start'),
  ev(3, 1, 'quality', 'agent_start'),
  ev(4, 2, 'lead', 'wake', { reason: 'start', mail: [], input: 'ticket' }),
  ev(5, 10, 'lead', 'delegation', { to: 'quality', task: 'hold CVX-2611' }),
  ev(6, 12, 'lead', 'idle', { ok: true, text: 'waiting' }),
  ev(7, 12, 'quality', 'wake', { reason: 'mail', mail: [5], input: 'hold CVX-2611' }),
  ev(8, 30, 'quality', 'report', { to: 'lead', text: 'held: C-1' }),
  ev(9, 31, 'quality', 'idle', { ok: true, text: 'done' }),
  ev(10, 31, 'lead', 'wake', { reason: 'mail', mail: [8], input: 'held' }),
  ev(11, 40, 'lead', 'finish', { summary: 'handled' }),
  ev(12, 41, 'lead', 'agent_end', { ok: true }),
  ev(13, 41, 'quality', 'agent_end', { ok: true }),
];

describe('a dispatch run on the timeline', () => {
  it('draws delegations and reports as messages between the lead and its agent', () => {
    const l = buildLanes(dispatched, team);
    const lead = l[0].marks.map((m) => m.label);
    expect(lead).toEqual(['Start', 'Started with the ticket', 'Delegate → quality', 'Woken by work or a message', 'Finished the run', 'Ended']);
    const report = l[1].marks.find((m) => m.seq === 8)!;
    expect([report.kind, report.label, report.to]).toEqual(['message', 'Report → lead', 'lead']);
    const span = spanOf(l)!;
    const layout = layoutTimeline(l, span, 1000);
    expect(layout.connectors.map((c) => [c.seq, c.from, c.to])).toEqual([[5, 0, 1], [8, 1, 0]]);
  });

  it('shows the times an agent waited for work as gaps', () => {
    const at = (s: number) => Date.parse(dispatched[0].at) + s * 1000;
    expect(gapsOf(dispatched, 'quality')).toEqual([{ start: at(1), end: at(12) }, { start: at(31), end: at(41) }]);
    expect(gapsOf(dispatched, 'lead')).toEqual([{ start: at(1), end: at(2) }, { start: at(12), end: at(31) }]);
    const live = dispatched.slice(0, 9);
    expect(gapsOf(live, 'quality').at(-1)).toEqual({ start: at(31), end: null });
    const l = buildLanes(dispatched, team);
    expect(layoutTimeline(l, spanOf(l)!, 1000).gaps.length).toBe(4);
  });

  it('draws no gaps for a run without wakes', () => {
    expect(gapsOf([runStart, ev(2, 1, 'lead', 'agent_start'), ev(3, 9, 'lead', 'agent_end')], 'lead')).toEqual([]);
  });
});
