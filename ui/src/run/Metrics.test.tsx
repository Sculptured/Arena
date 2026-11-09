import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import MetricsPanel, { costText, estimateText, overheadLines } from './Metrics';
import { runnerLine } from '../pages/Run';
import type { Event, Metrics } from '../types';

const cost = (usd: number | null, label: string, as_of: string | null = null) => ({ usd, label, source: null, as_of });

const metrics = (over: Partial<Metrics> = {}): Metrics => ({
  measured_s: 12, overhead_ms: { save: 1500, record: 500 }, overhead_s: 2, overhead_recorded: true, estimated_s: 10,
  tokens: 150, cost: cost(0.5, 'reported'),
  agents: [{ agent: 'a', kind: 'open', model: 'm', provider: 'anthropic', tokens: 150, prompt_tokens: 100, completion_tokens: 50, cost: cost(0.5, 'reported') }],
  ...over,
});

const runStart = (payload: object): Event[] => [{ branch_id: 'b', seq: 1, at: '', agent: null, kind: 'run_start', payload }];

describe('metrics', () => {
  it('lists the overhead that took time, largest first', () => {
    expect(overheadLines({ record: 500, save: 1500, gate: 0 })).toEqual([['saves', '1.50 s'], ['recording', '0.50 s']]);
  });
  it('labels every cost and never invents one', () => {
    expect(costText(cost(1.5, 'list-price estimate', '2026-11-04'))).toBe('$1.50 (list-price estimate, prices of 2026-11-04)');
    expect(costText(cost(null, 'unknown'))).toBe('unknown');
    expect(costText(cost(0, 'no API charge'))).toBe('no API charge');
  });
  it('estimates production as measured minus overhead, or says why not', () => {
    expect(estimateText(metrics())).toBe('10.0 s (12.0 s measured − 2.00 s Shakedown overhead)');
    expect(estimateText(metrics({ estimated_s: null, overhead_recorded: false }))).toContain('not estimated');
  });
  it('shows each agent with its model and tokens', () => {
    const html = renderToStaticMarkup(<MetricsPanel m={metrics()} />);
    expect(html).toContain('Anthropic · m');
    expect(html).toContain('150 (100 / 50)');
  });
  it('says how the agents ran, old runs included', () => {
    expect(runnerLine(runStart({ task_id: '1' }))).toContain('recorded before runners');
    expect(runnerLine(runStart({ task_id: '1', runner: 'concurrent' }))).toContain('at once');
    expect(runnerLine(runStart({ task_id: '1', runner: 'dispatch' }))).toContain('A lead dispatched');
    expect(runnerLine([])).toBe('');
  });
});
