import { describe, expect, it } from 'vitest';
import { actionsOf } from './actions';
import { events } from './fixtures';
import { actionsPerBucket, bucketFor, cumulative, latencies, modelCalls, perMinute } from './trends';

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

  it('picks_a_bucket_for_about_two_dozen_points_and_gives_rates_per_minute', () => {
    expect(bucketFor(actionsOf(events))).toBe(5_000);
    expect(bucketFor([])).toBe(60_000);
    expect(perMinute([1, 2], 15_000)).toEqual([4, 8]);
  });

  it('lists_the_tool_calls_whose_duration_was_recorded', () => {
    expect(latencies(actionsOf(events)).map((l) => l.ms)).toEqual([120, 1500]);
  });
});
