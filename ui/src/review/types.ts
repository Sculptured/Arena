// AI review, as arenad stores it (`GET/POST /api/branches/{id}/review`, `/api/compare/review`,
// `/api/suites/compare/review`).

export type Severity = 'safety' | 'goal' | 'efficiency';
export type SuggestionKind = 'prompt' | 'team' | 'tool' | 'model' | 'process';

export interface WentWrong {
  title: string;
  why: string;
  steps: number[];
  severity: Severity;
  /** Every step it cited was not in the run, so arenad dropped them. */
  unverified?: boolean;
}

export interface Suggestion {
  change: string;
  why: string;
  kind: SuggestionKind;
  agent?: string;
  /** Sentences to add to the agent's prompt. */
  prompt_text?: string;
  /** The step to rerun from. */
  step?: number;
}

export interface RunReview {
  verdict: string;
  went_wrong: WentWrong[];
  suggestions: Suggestion[];
  went_well: string[];
  confidence: 'high' | 'medium' | 'low' | string;
}

export interface Difference {
  title: string;
  a: string;
  b: string;
  why_it_matters: string;
  /** Steps (two runs) or branch ids of example runs (two suites). */
  steps_a: (number | string)[];
  steps_b: (number | string)[];
  unverified?: boolean;
}

export interface CompareReview {
  headline: string;
  differences: Difference[];
  which_is_better: 'a' | 'b' | 'mixed';
  why: string;
}

export interface StoredReview<R = RunReview | CompareReview> {
  key: string;
  kind: 'run' | 'compare' | 'suites';
  status: 'running' | 'done' | 'error';
  model: string;
  review: R | null;
  issues: string[];
  error: string | null;
  cost_usd: number | null;
  cost_source: string | null;
  tokens: number | null;
  seconds: number | null;
  digest_bytes: number | null;
  created_at: string;
  finished_at: string | null;
}

export interface ReviewSettings {
  enabled: boolean;
  model: string;
  why?: string;
}
