import { describe, expect, it } from 'vitest';
import { changedFields, rowKeys, jsonDiff, lanes, summarize } from './timeline';
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
    const saved = lanes([ev(1, 'triage', 'tool_call', { name: 'start_line', writes: ['line/1'], snapshot: null, saves: { plant: 's2' } })], team, null);
    expect(saved[0].steps[0].snapshot).toBe(true);
  });
});

describe('jsonDiff', () => {
  it('marks changed lines and absent sides', () => {
    const d = jsonDiff({ a: 1, b: 2 }, { a: 1, b: 3 });
    expect(d.filter((l) => l.op !== ' ').map((l) => `${l.op}${l.text.trim()}`)).toEqual(['-"b": 2', '+"b": 3']);
    expect(jsonDiff(undefined, { a: 1 })[0]).toEqual({ op: '-', text: '(absent)' });
  });
});

const end = (seq: number, p: object) => ev(seq, 'triage', 'agent_end', p);
const br = { start_seq: null, created_at: '2026-01-01T00:00:00Z', ended_at: '2026-01-01T00:00:10Z', status: 'passed' as const };

describe('summarize', () => {
  it('sums agent_end, single source, seconds from timestamps', () => {
    const s = summarize(br, [end(2, { tokens: 10, cost_usd: 0.5, cost_source: 'openrouter' }), end(5, { tokens: 5, cost_usd: 0.25, cost_source: 'openrouter' })]);
    expect(s).toMatchObject({ seconds: 10, tokens: 15, cost_usd: 0.75, cost_source: 'openrouter', scope: null, live: false });
  });
  it('mixed sources and null cost', () => {
    const m = summarize(br, [end(2, { tokens: 1, cost_usd: 1, cost_source: 'openrouter' }), end(3, { tokens: 1, cost_usd: 1, cost_source: 'api_equivalent_estimate' })]);
    expect(m.cost_source).toBe('mixed');
    const n = summarize(br, [end(2, { tokens: 1, cost_usd: null, cost_source: null }), end(3, { tokens: 1, cost_usd: 1, cost_source: 'openrouter' })]);
    expect(n.cost_usd).toBeNull();
  });
  it('forked counts only own events; running uses now', () => {
    const s = summarize({ ...br, start_seq: 4, ended_at: null, status: 'running' },
      [end(2, { tokens: 100, cost_usd: 9, cost_source: 'openrouter' }), end(6, { tokens: 7, cost_usd: 1, cost_source: 'openrouter' })],
      Date.parse('2026-01-01T00:00:03Z'));
    expect(s).toMatchObject({ tokens: 7, cost_usd: 1, seconds: 3, live: true, scope: 'since fork at step 4' });
  });
});

describe('changedFields', () => {
  const agents = team.agents.map((a) => ({ ...a, model: 'sonnet', prompt: 'p' }));
  it('ignores whitespace prompt edits and unchanged values', () => {
    expect(changedFields(agents, { triage: { prompt: ' p ' }, resolver: { model: 'sonnet', kind: 'claude_code' } })).toEqual({});
  });
  it('a cleared model that was set is an explicit clear', () => {
    expect(changedFields(agents, { triage: { model: '  ' } })).toEqual({ triage: { model: '' } });
  });
  it('blank model on an unset model is no change', () => {
    const unset = agents.map((a) => ({ ...a, model: null }));
    expect(changedFields(unset, { triage: { model: '' } })).toEqual({});
  });
  it('a mode change with a cleared model box sends just the mode', () => {
    expect(changedFields(agents, { triage: { kind: 'codex', model: '' } })).toEqual({ triage: { kind: 'codex' } });
    expect(changedFields(agents, { triage: { kind: 'codex', model: 'gpt-x' } })).toEqual({ triage: { kind: 'codex', model: 'gpt-x' } });
  });
  it('keeps real changes, trimmed', () => {
    expect(changedFields(agents, { triage: { model: ' opus ', prompt: 'q ' , kind: 'codex' } })).toEqual({ triage: { model: 'opus', prompt: 'q', kind: 'codex' } });
  });
});

describe('rowKeys', () => {
  it('drops wildcard reads and dedups, writes first', () => {
    expect(rowKeys(['orders/#W1'], ['orders/*', 'users/u1', 'orders/#W1'])).toEqual({ keys: ['orders/#W1', 'users/u1'], more: 0 });
  });
  it('caps at 50 and reports the rest', () => {
    const reads = Array.from({ length: 53 }, (_, i) => `products/p${i}`);
    const r = rowKeys([], reads);
    expect(r.keys).toHaveLength(50);
    expect(r.more).toBe(3);
  });
});
