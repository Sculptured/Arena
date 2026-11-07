import { describe, expect, it } from 'vitest';
import { buildLanes, spanOf } from '../control/lanes';
import { ev, events, runStart, team } from '../control/fixtures';
import { runStats } from './runStats';
import { diffCounts, jsonDiff } from './timeline';
import { layoutTimeline, shapeOf, shortLabel, stepOrder, ticksOf } from './timelineLayout';
import { spanLength, traceOf, turnsOf } from './waterfall';

describe('timeline layout', () => {
  const lanes = buildLanes(events, team);
  const span = spanOf(lanes)!;
  it('reads hollow, writes solid, approvals a diamond, refusals the alarm shape', () => {
    const q = lanes[1].marks;
    expect(q.filter((m) => m.kind === 'action' || m.kind === 'interlock').map(shapeOf)).toEqual(['read', 'write', 'gate-write', 'abnormal']);
    expect(shortLabel(q.find((m) => m.seq === 7)!)).toBe('ERP');
    expect(shortLabel(lanes[0].marks.find((m) => m.kind === 'message')!)).toBe('→ quality');
  });
  it('places marks by time, draws messages between lanes and labels only marks with room', () => {
    const l = layoutTimeline(lanes, span, 1000);
    const xs = l.placed.filter((p) => p.lane === 1).map((p) => p.x);
    expect(xs).toEqual([...xs].sort((a, b) => a - b));
    expect(l.connectors).toEqual([{ seq: 4, x: l.placed.find((p) => p.mark.seq === 4)!.x, from: 0, to: 1 }]);
    const tight = layoutTimeline(lanes, span, 120).placed.filter((p) => p.labelled).length;
    expect(tight).toBeLessThan(l.placed.filter((p) => p.labelled).length);
    expect(layoutTimeline(lanes, span, 1000, false).placed.some((p) => p.mark.kind === 'model')).toBe(false);
  });
  it('ticks on round seconds and orders the steps for the keyboard', () => {
    expect(ticksOf(span, 1000).map((t) => t.label)).toEqual(['0 s', '10 s', '20 s', '30 s', '40 s', '50 s', '60 s', '70 s', '80 s', '90 s']);
    expect(stepOrder(lanes, false)).toEqual([2, 4, 5, 6, 7, 8, 9, 11, 12]);
  });
});

describe('trace waterfall', () => {
  it('cuts an agent\'s steps into turns at each model call', () => {
    expect(turnsOf(events, 'quality').map((t) => t.map((e) => e.seq))).toEqual([[5, 6, 7, 8, 9], [10, 11]]);
  });
  it('shows the turn of a step, each step ending when recorded and starting its duration before', () => {
    const t = traceOf(events, 7)!;
    expect(t.title).toBe('before its first model call');
    const open = t.spans.find((s) => s.seq === 7)!;
    expect(open.end - open.start).toBe(1500);
    expect(t.spans[0]).toMatchObject({ kind: 'turn', start: 0, abnormal: true });
    expect(t.spans.find((s) => s.seq === 9)!.abnormal).toBe(true);
    expect(traceOf(events, 12)).toBeNull();
    expect(traceOf(events, 11)!.title).toBe('turn 1 of 1');
  });
  it('lists arena\'s own work in a step under it', () => {
    const evs = [runStart, ev(2, 1, 'quality', 'tool_call', { name: 'get_result', duration_ms: 10, overhead_ms: { gate: 4, record: 2, wait: 0.001 } })];
    expect(traceOf(evs, 2)!.spans.filter((s) => s.kind === 'overhead').map((s) => [s.label, s.end - s.start])).toEqual([['arena · gate check', 4], ['arena · recording', 2]]);
    expect([spanLength(1400), spanLength(120), spanLength(4), spanLength(0.06)]).toEqual(['1.4 s', '120 ms', '4.0 ms', '0.06 ms']);
  });
});

describe('run stats and diffs', () => {
  it('counts own steps, reads, writes and abnormal actions, and reads the check', () => {
    expect(runStats(events, null)).toEqual({ steps: 11, reads: 1, writes: 3, abnormal: 1, check: { passed: false, ok: 0, total: 0 } });
    expect(runStats(events, 8)).toMatchObject({ steps: 5, reads: 0, writes: 2, abnormal: 1 });
  });
  it('counts the lines a diff adds and removes', () => {
    expect(diffCounts(jsonDiff({ a: 1 }, { a: 2, b: 3 }))).toEqual({ added: 2, removed: 1 });
  });
});
