import { describe, expect, it } from 'vitest';
import { duration, fmt } from '@/lib/format';
import type { RunOutcome, RunRow, Spread, SuiteReport } from '../control/types';
import type { Team } from '../types';
import { METRICS, bestOf, checkLabel, checkMatrix, columnOf, setupTeam, setupsProblem, wilson } from './model';

const outcome = (passed: boolean, over: Partial<RunOutcome> = {}): RunOutcome => ({
  status: passed ? 'passed' : 'failed', error: null, checks: [], scored: true,
  checks_passed: passed ? 2 : 1, checks_total: 2, safety_violations: 0, denials: 0, flags: 0,
  measured_s: 1, estimated_s: 1, tokens: 1, cost_usd: null, peak_context: null, ...over,
});

const row = (repeat: number, status: RunRow['status'], o: RunOutcome | null): RunRow =>
  ({ task_id: 't', repeat, run_id: o ? `r${repeat}` : null, branch_id: o ? `b${repeat}` : null, status, error: null, outcome: o });

const spread = (mean: number | null): Spread => ({ n: 0, unknown: 0, min: mean, max: mean, mean });

const team: Team = { id: 'team', name: 'Planners', agents: [
  { id: 'lead', name: 'Lead', kind: 'claude_code', model: 'sonnet', prompt: 'Plan.' },
  { id: 'helper', name: 'Helper', kind: 'codex', model: null, prompt: 'Help.' },
] };

const report = (id: string, rows: RunRow[], passed: number, over: Partial<SuiteReport['summary']> = {}): SuiteReport => ({
  id, name: `Setup ${id}`, team, tasks: ['t'], repeats: rows.length, runner: 'chain', benchmark: 'b', created_at: '', ended_at: null, error: null,
  status: 'running', finished_s: null, runs: rows,
  checks: [
    { task_id: 't', name: 'order_delivered', safety: false, passed, runs: 3, pass_rate: passed / 3 },
    { task_id: 't', name: 'no_split_purchase', safety: true, passed: 3, runs: 3, pass_rate: 1 },
  ],
  summary: {
    runs: rows.length, ended: 3, scored: 3, unscored: 0, passed, pass_rate: passed / 3, score: null, safety_violations: 0, runs_with_violation: 0,
    denials: 0, flags: 0, runs_with_denial: 0, runs_with_flag: 0,
    checks_passed: spread(1.5), measured_s: spread(120), estimated_s: spread(null), tokens: spread(null), cost_usd: spread(0.5), peak_context: spread(null), ...over,
  },
  spread: [],
});

describe('experiment columns', () => {
  const a = report('A', [row(1, 'passed', outcome(true)), row(2, 'failed', outcome(false, { denials: 2, safety_violations: 1 })), row(3, 'passed', outcome(true)), row(4, 'running', null), row(5, 'pending', null)], 2, { denials: 2, safety_violations: 1 });
  const b = report('B', [row(1, 'passed', outcome(true, { flags: 1 })), row(2, 'passed', outcome(true)), row(3, 'passed', outcome(true))], 3, { flags: 1, cost_usd: spread(0.8) });

  it('gives each run a cell: passed, failed, running, not run yet; scored ones link to the run', () => {
    const c = columnOf(a);
    expect(c.runs.map((r) => r.state)).toEqual(['passed', 'failed', 'passed', 'running', 'pending']);
    expect(c.runs[1]).toMatchObject({ checks: '1/2', denials: 2, violations: 1, href: '/runs/r2/b2' });
    expect(c.runs[1].title).toBe('Run 2 · Failed · 1/2 checks · 1 safety violation · 2 denials');
    expect(c.runs[4].href).toBeNull();
    expect(c.team).toBe('2 agents · Claude Code, Codex · Sonnet, Default model');
  });

  it('puts a 95% range around the pass rate that narrows with more runs', () => {
    const [lo, hi] = wilson(3, 5)!;
    expect(lo).toBeCloseTo(0.231, 2);
    expect(hi).toBeCloseTo(0.882, 2);
    const [lo2, hi2] = wilson(30, 50)!;
    expect(hi2 - lo2).toBeLessThan(hi - lo);
    expect(wilson(0, 0)).toBeNull();
    expect(wilson(5, 5)![1]).toBe(1);
  });

  it('marks the best setup per figure, and none when equal or when better is not a direction', () => {
    const cols = [columnOf(a), columnOf(b)];
    const by = (id: string) => METRICS.find((m) => m.id === id)!;
    expect([...bestOf(cols, by('cost'))]).toEqual([0]);
    expect([...bestOf(cols, by('denials'))]).toEqual([1]);
    expect([...bestOf(cols, by('time'))]).toEqual([]);
    expect([...bestOf(cols, by('flags'))]).toEqual([]);
    expect([...bestOf([cols[0]], by('cost'))]).toEqual([]);
    expect(by('violations').abnormal!(cols[0])).toBe(true);
    expect(by('checks').text(cols[0])).toBe('1.5 of 2');
  });

  it('lines the checks up side by side, goal checks first, and says which every run passed', () => {
    const rows = checkMatrix([a, b]);
    expect(rows.map((r) => [r.name, r.safety, r.clean])).toEqual([['order_delivered', false, false], ['no_split_purchase', true, true]]);
    expect(rows[0].cells).toEqual([{ passed: 2, runs: 3 }, { passed: 3, runs: 3 }]);
    expect(checkLabel('no_split_purchase')).toBe('No split purchase');
  });
});

describe('new experiment setups', () => {
  it('puts every Claude Code agent on the chosen model and adds the rules to every prompt', () => {
    const t = setupTeam(team, { model: 'haiku', rules: ' Escalate spend over the limit. ' });
    expect(t.agents.map((x) => x.model)).toEqual(['haiku', null]);
    expect(t.agents[0].prompt).toBe('Plan.\n\nDeployment rules:\nEscalate spend over the limit.');
    expect(setupTeam(team, { model: '', rules: '' })).toEqual(team);
  });
  it('needs a distinct name for every setup', () => {
    const d = (name: string) => ({ key: name, name, teamId: 's:team', model: '', rules: '' });
    expect(setupsProblem([d('A'), d('B')])).toBeNull();
    expect(setupsProblem([d('A'), d(' ')])).toBe('Every setup needs a name.');
    expect(setupsProblem([d('A'), d('A ')])).toBe('Two setups are called “A”.');
  });
});

describe('figures', () => {
  it('formats money, tokens and time one way', () => {
    expect([fmt(0.0412, 'usd'), fmt(0.56, 'usd'), fmt(12_400, 'tok'), fmt(3, 's'), fmt(null, 'n')]).toEqual(['$0.041', '$0.56', '12.4k', '3.0 s', '–']);
    expect([duration(45.4), duration(188), duration(null)]).toEqual(['45 s', '3.1 min', '–']);
  });
});
