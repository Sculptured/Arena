// Mirrors arenad's experiments API (`api/experiments.rs`): an experiment is one scenario run by
// 1–4 agent setups, N times each; each setup is stored as a suite, so its report is a SuiteReport.
import type { SuiteReport, SuiteStatus } from '../control/types';
import type { ScenarioChoice } from '../scenario/types';
import type { Runner, Team } from '../types';

/** A setup's line in the list: its suite, name, team and how many of its runs ended and passed. */
export interface SetupLine {
  suite_id: string;
  name: string;
  team_name: string;
  runner: Runner;
  runs: number;
  ended: number;
  passed: number;
}

interface ExperimentBase {
  id: string;
  name: string;
  /** The scenario's task (a suite from before experiments may have several). */
  tasks: string[];
  benchmark: string;
  /** Runs per setup. */
  runs: number;
  created_at: string;
  status: SuiteStatus;
  /** A suite from before experiments, shown as a one-setup experiment. */
  legacy: boolean;
}

export interface ListedExperiment extends ExperimentBase {
  setups: SetupLine[];
}

export interface Experiment extends ExperimentBase {
  ended_at: string | null;
  error: string | null;
  setups: SuiteReport[];
}

export interface NewSetup {
  name: string;
  team: Team;
  runner: Runner;
}

export interface NewExperiment {
  name?: string;
  task: string;
  scenario?: ScenarioChoice;
  runs: number;
  setups: NewSetup[];
}
