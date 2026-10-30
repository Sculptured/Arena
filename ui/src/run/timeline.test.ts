import { describe, expect, it } from 'vitest';
import { jsonDiff, lanes } from './timeline';
import type { Event, EventKind, Team } from '../types';

const team: Team = {
  id: 't', name: 't',
  agents: [
    { id: 'triage', name: 'Triage', kind: 'claude_code', model: null, prompt: '' },
    { id: 'resolver', name: 'Resolver', kind: 'claude_code', model: null, prompt: '' },
  ],
};
const ev = (seq: number, agent: string | null, kind: EventKind, payload: unknown = {}): Event =>
  ({ branch_id: 'b', seq, at: '', agent, kind, payload });

const events: Event[] = [
  ev(0, null, 'run_start'),
  ev(1, 'triage', 'agent_start'),
  ev(2, 'triage', 'tool_call', { name: 'get_order', writes: [], reads: ['orders/1'] }),
  ev(3, 'triage', 'handoff', { note: 'x' }),
  ev(4, 'resolver', 'agent_start'),
  ev(5, 'resolver', 'model_call'),
  ev(6, 'resolver', 'tool_call', { name: 'cancel_order', writes: ['orders/1'], reads: [], snapshot: 's1' }),
  ev(7, null, 'check', { passed: true }),
];

describe('lanes', () => {
  it('lanes_group_by_agent_in_chain_order', () => {
    const l = lanes(events, team, null);
    expect(l.map((x) => x.agent)).toEqual(['triage', 'resolver', 'check']);
    expect(l[0].steps.map((s) => s.seq)).toEqual([1, 2, 3]);
    expect(l[1].steps.map((s) => s.seq)).toEqual([4, 5, 6]);
    expect(l[2].steps.map((s) => s.kind)).toEqual(['check']);
    expect(l[0].steps[1].label).toBe('get_order');
  });
  it('inherited_steps_marked_below_start_seq', () => {
    const l = lanes(events, team, 5);
    const flags = l.flatMap((x) => x.steps).map((s) => [s.seq, s.inherited]);
    expect(flags).toEqual([[1, true], [2, true], [3, true], [4, true], [5, false], [6, false], [7, false]]);
    expect(lanes(events, team, null).flatMap((x) => x.steps).some((s) => s.inherited)).toBe(false);
  });
  it('write_steps_flagged', () => {
    const steps = lanes(events, team, null).flatMap((x) => x.steps);
    const s = (n: number) => steps.find((x) => x.seq === n)!;
    expect(s(6)).toMatchObject({ wrote: true, snapshot: true });
    expect(s(2)).toMatchObject({ wrote: false, snapshot: false });
  });
});

describe('jsonDiff', () => {
  it('marks changed lines and absent sides', () => {
    const d = jsonDiff({ a: 1, b: 2 }, { a: 1, b: 3 });
    expect(d.filter((l) => l.op !== ' ').map((l) => `${l.op}${l.text.trim()}`)).toEqual(['-"b": 2', '+"b": 3']);
    expect(jsonDiff(undefined, { a: 1 })[0]).toEqual({ op: '-', text: '(absent)' });
  });
});
