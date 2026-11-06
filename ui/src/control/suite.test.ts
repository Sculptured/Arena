import { describe, expect, it } from 'vitest';
import { change, checkRows, fmt, grid, pct, pts, runMark, spreadLine, verdictMark } from './suite';
import type { RunOutcome, RunRow, Spread, SuiteReport } from './types';

const outcome = (passed: boolean, unsafe: number): RunOutcome => ({
  status: passed ? 'passed' : 'failed', error: null, checks: [], scored: true,
  checks_passed: passed ? 2 : 1, checks_total: 2, safety_violations: unsafe,
  measured_s: 1, estimated_s: 1, tokens: 1, cost_usd: null, peak_context: null,
});

const row = (task_id: string, repeat: number, status: RunRow['status'], o: RunOutcome | null): RunRow =>
  ({ task_id, repeat, run_id: o ? `r${task_id}${repeat}` : null, branch_id: o ? `b${task_id}${repeat}` : null, status, error: null, outcome: o });

const spread: Spread = { n: 0, unknown: 0, min: null, max: null, mean: null };
const report = {
  id: 's', name: 's', team: { id: 't', name: 'T', agents: [] }, tasks: ['base', 'short'], repeats: 2,
  runner: 'external', benchmark: 'pharma', created_at: '', ended_at: null, error: null,
  status: 'running', finished_s: null,
  runs: [row('short', 1, 'failed', outcome(false, 1)), row('base', 2, 'passed', outcome(true, 0)),
    row('base', 1, 'passed', outcome(true, 0)), row('short', 2, 'pending', null)],
  checks: [
    { task_id: 'base', name: 'no_unsafe_ship', safety: true, passed: 2, runs: 2, pass_rate: 1 },
    { task_id: 'base', name: 'deviation_opened', safety: false, passed: 2, runs: 2, pass_rate: 1 },
  ],
  summary: { runs: 4, ended: 3, scored: 3, unscored: 0, passed: 2, pass_rate: 2 / 3, score: 5 / 6, safety_violations: 1, runs_with_violation: 1,
    checks_passed: spread, measured_s: spread, estimated_s: spread, tokens: spread, cost_usd: spread, peak_context: spread },
  spread: [],
} as unknown as SuiteReport;

describe('suite display', () => {
  it('lays_out_tasks_by_repeats_in_order', () => {
    const g = grid(report);
    expect(g.map((r) => [r.task, r.cells.map((c) => c.repeat), r.passed, r.scored, r.violations])).toEqual([
      ['base', [1, 2], 2, 2, 0],
      ['short', [1, 2], 0, 1, 1],
    ]);
    const c = g[1].cells[0];
    expect([c.mark.word, c.mark.abnormal, c.checks, c.unsafe, c.branchId]).toEqual(['failed', true, '1/2', true, 'bshort1']);
    expect([g[1].cells[1].mark.word, g[1].cells[1].checks]).toEqual(['pending', '']);
  });

  it('shows_status_and_verdicts_as_glyph_and_word_with_colour_only_for_abnormal', () => {
    expect(runMark('passed')).toEqual({ glyph: '✓', word: 'passed', abnormal: false });
    expect(runMark('infra_error').abnormal).toBe(true);
    expect(verdictMark('regressed')).toEqual({ glyph: '▼', word: 'regressed', abnormal: true });
    expect(verdictMark('improved').abnormal).toBe(false);
  });

  it('lists_goal_checks_before_safety_checks', () => {
    expect(checkRows(report.checks, 'base').map((c) => c.name)).toEqual(['deviation_opened', 'no_unsafe_ship']);
  });

  it('formats_rates_changes_and_spreads', () => {
    expect([pct(2 / 3), pct(null), pts(0.25), pts(-0.1), pts(0)]).toEqual(['67%', '–', '+25 pts', '−10 pts', '±0']);
    expect(spreadLine({ n: 2, unknown: 0, min: 2, max: 4, mean: 3 }, 's')).toBe('3.0 s (2.0 s–4.0 s)');
    expect(spreadLine({ n: 1, unknown: 1, min: 0.01, max: 0.01, mean: 0.01 }, 'usd')).toBe('$0.0100, 1 unknown');
    expect(spreadLine({ n: 0, unknown: 2, min: null, max: null, mean: null }, 'usd')).toBe('unknown (2)');
    expect([fmt(12345, 'tok'), change(-300, 'tok'), change(1.25, 's'), change(null, 's'), change(-0.01, 's')]).toEqual(['12.3k', '−300', '+1.3 s', '–', '±0']);
  });
});
