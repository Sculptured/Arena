// A scenario chosen per run (summit demo contract, sections A and B): the knobs the benchmark's
// `describe` offers as `scenario_options`, and the choice sent on POST /api/runs and /api/suites.

/** When a disruption fires: before answering the agent tool call that crosses `after_calls`. */
export interface DisruptionWhen {
  after_calls: number;
  /** Count only calls of this tool. */
  tool?: string;
}

/** What a disruption does (`lane_capacity`, `lane_unavailable`, `transit_delay`, `reject_next`, `result_fails`, ...). */
export interface DisruptionDo {
  kind: string;
  [field: string]: unknown;
}

/** A disruption scheduled inside a simulated part. */
export interface Disruption {
  id: string;
  when: DisruptionWhen;
  do: DisruptionDo;
  /** Plain words, e.g. "Tundra Medical Courier loses capacity after its first quote." */
  text: string;
}

export interface ParamSpec {
  name: string;
  label: string;
  kind: 'int' | 'choice';
  choices?: (string | number)[];
  min?: number;
  max?: number;
  default: string | number;
}

/** One disruption the builder offers; `template`'s strings may hold `{param}` placeholders. */
export interface DisruptionOption {
  id: string;
  label: string;
  description: string;
  part: string;
  default?: boolean;
  params: ParamSpec[];
  template: unknown;
}

/** `describe.scenario_options`. */
export interface ScenarioOptions {
  /** The tasks the disruptions apply to. */
  base: string[];
  disruptions: DisruptionOption[];
}

export type ParamValue = string | number;

/** The scenario a run or suite is started with. */
export interface ScenarioChoice {
  disruptions: { id: string; params: Record<string, ParamValue> }[];
}

/** `run_start.scenario`: the scenario as arenad expanded it. */
export interface RecordedScenario {
  disruptions?: Disruption[];
}
