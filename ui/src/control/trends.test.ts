import { describe, expect, it } from 'vitest';
import { actionsOf } from './actions';
import { events } from './fixtures';
import { actionsPerBucket, cumulative, modelCalls, sparkPoints } from './trends';

describe('trends', () => {
  it('counts_actions_per_minute_per_system_kind', () => {
    const b = actionsPerBucket(actionsOf(events));
    expect(b.starts.length).toBe(2);
    expect(b.series).toEqual([
      { name: 'LIMS', values: [1, 0] },
      { name: 'ERP', values: [1, 1] },
      { name: 'approvals', values: [0, 1] },
    ]);
  });

  it('gives_the_context_of_each_model_call_and_running_totals_unknown_once_a_cost_is', () => {
    const calls = modelCalls(events);
    expect(calls.map((c) => [c.agent, c.context])).toEqual([['lead', 1000], ['quality', 3000]]);
    const c = cumulative(calls);
    expect(c.tokens).toEqual([1050, 4150]);
    expect(c.cost).toEqual([0.01, null]);
  });

  it('draws_a_sparkline_in_its_box', () => {
    expect(sparkPoints([0, 5, 10], 100, 10)).toBe('0.0,10.0 50.0,5.0 100.0,0.0');
    expect(sparkPoints([], 100, 10)).toBe('');
  });
});
