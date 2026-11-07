// Mirrors arenad's suite/ (report.rs, compare.rs, outcome.rs) and world/systems.rs (serde snake_case).
import type { ScenarioChoice } from '../scenario/types';
import type { BranchStatus, Runner, Team } from '../types';

/** What kind of system a part is (`describe.system`); absent when its integration does not say. */
export interface SystemInfo {
  kind: string;
  product: string;
  stands_in_for?: string;
  simulated?: boolean;
}

export type Effect = 'read' | 'write';

/** A tool of the world as `run_start.tools` records it; a benchmark's own tool has no fixed part. */
export interface ToolInfo {
  effect: Effect;
  part?: string;
}

export interface NamedCheck {
  name: string;
  passed: boolean;
  detail?: string;
  safety?: boolean;
}

export interface RunOutcome {
  status: BranchStatus;
  error: string | null;
  checks: NamedCheck[];
  scored: boolean;
  checks_passed: number;
  checks_total: number;
  safety_violations: number;
  measured_s: number;
  estimated_s: number | null;
  tokens: number;
  cost_usd: number | null;
  peak_context: number | null;
}

/** `pending`, `starting`, `error` (could not start), `missing` (run deleted), or the branch's status. */
export type RunRowStatus = BranchStatus | 'pending' | 'starting' | 'error' | 'missing';

export interface RunRow {
  task_id: string;
  repeat: number;
  run_id: string | null;
  branch_id: string | null;
  status: RunRowStatus;
  error: string | null;
  outcome: RunOutcome | null;
}

export interface CheckRate {
  task_id: string;
  name: string;
  safety: boolean;
  passed: number;
  runs: number;
  pass_rate: number;
}

export interface Spread {
  n: number;
  unknown: number;
  min: number | null;
  max: number | null;
  mean: number | null;
}

export interface Figures {
  checks_passed: Spread;
  measured_s: Spread;
  estimated_s: Spread;
  tokens: Spread;
  cost_usd: Spread;
  peak_context: Spread;
}

export interface Summary extends Figures {
  runs: number;
  ended: number;
  scored: number;
  unscored: number;
  passed: number;
  pass_rate: number | null;
  score: number | null;
  safety_violations: number;
  runs_with_violation: number;
}

export interface TaskSpread extends Figures {
  task_id: string;
  runs: number;
}

export type SuiteStatus = 'running' | 'finished' | 'stopped';

export interface Suite {
  id: string;
  name: string;
  team: Team;
  tasks: string[];
  repeats: number;
  runner: Runner;
  benchmark: string;
  created_at: string;
  ended_at: string | null;
  error: string | null;
}

export interface SuiteReport extends Suite {
  status: SuiteStatus;
  finished_s: number | null;
  runs: RunRow[];
  checks: CheckRate[];
  summary: Summary;
  spread: TaskSpread[];
}

export interface ListedSuite extends Suite {
  status: SuiteStatus;
  runs: number;
  ended: number;
  passed: number;
}

export type Verdict = 'improved' | 'regressed' | 'same' | 'incomparable';

export interface Delta {
  a: number | null;
  b: number | null;
  change: number | null;
  verdict: Verdict;
}

export interface CheckDelta extends Delta {
  task_id: string;
  name: string;
  safety: boolean;
}

export interface CompareSide {
  id: string;
  name: string;
  team: string;
  status: SuiteStatus;
  scored: number;
}

export interface SuiteComparison {
  a: CompareSide;
  b: CompareSide;
  verdict: Verdict;
  checks: CheckDelta[];
  pass_rate: Delta;
  score: Delta;
  safety: Delta;
  measured_s: Delta;
  estimated_s: Delta;
  tokens: Delta;
  cost_usd: Delta;
  peak_context: Delta;
}

export interface NewSuite {
  name?: string;
  team: Team;
  tasks: string[];
  repeats: number;
  runner: Runner;
  /** Disruptions every run of the suite is started with. */
  scenario?: ScenarioChoice;
}
