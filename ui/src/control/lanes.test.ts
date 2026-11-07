import { describe, expect, it } from 'vitest';
import { CHECK_LANE, buildLanes, formatDuration, position, spanOf } from './lanes';
import { ev, events, runStart, team } from './fixtures';

describe('lanes', () => {
  it('builds_one_lane_per_agent_in_team_order_then_the_check', () => {
    const l = buildLanes(events, team);
    expect(l.map((x) => x.id)).toEqual(['lead', 'quality', CHECK_LANE]);
    expect(l[0].marks.map((m) => m.label)).toEqual(['start', 'model call', 'message → quality']);
  });

  it('marks_approvals_and_refusals_as_interlocks_and_keeps_writes_apart_from_reads', () => {
    const q = buildLanes(events, team)[1].marks.filter((m) => m.kind === 'action' || m.kind === 'interlock');
    expect(q.map((m) => [m.label, m.kind, m.effect, m.outcome])).toEqual([
      ['LIMS read', 'action', 'read', 'ok'],
      ['ERP write', 'action', 'write', 'ok'],
      ['approvals write', 'interlock', 'write', 'ok'],
      ['ERP write', 'interlock', 'write', 'refused'],
    ]);
    expect(q[3].detail).toContain('no approval');
  });

  it('places_marks_by_time_and_formats_durations', () => {
    const l = buildLanes(events, team);
    const span = spanOf(l)!;
    expect(span.end - span.start).toBe(94_000);
    const read = l[1].marks.find((m) => m.label === 'LIMS read')!;
    expect(position(read.at, span)).toBeCloseTo(9 / 94);
    expect([formatDuration(120), formatDuration(1500), formatDuration(null)]).toEqual(['120 ms', '1.5 s', '–']);
  });

  it('inherited_steps_of_a_fork_are_marked', () => {
    const l = buildLanes(events, team, 7);
    const q = l[1].marks;
    expect(q.find((m) => m.seq === 6)!.inherited).toBe(true);
    expect(q.find((m) => m.seq === 7)!.inherited).toBe(false);
  });

  it('marks_the_steps_the_world_was_saved_after_including_legacy_snapshots', () => {
    const saved = buildLanes([runStart, ev(2, 1, 'quality', 'tool_call', { name: 'open_deviation', saves: { erp: 's2' } }),
      ev(3, 2, 'quality', 'tool_call', { name: 'hold_batch', snapshot: 's3' }), ev(4, 3, 'quality', 'tool_call', { name: 'get_result' })], team);
    expect(saved[1].marks.map((m) => [m.seq, m.saved])).toEqual([[2, true], [3, true], [4, false]]);
  });
});
