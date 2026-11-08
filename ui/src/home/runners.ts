// How each runner is named where a run is set up: the Advanced choice on the Run screen, the
// suite dialog and the team picker.
import type { Runner } from '../types';

/** The runners a person picks from, in the order offered; `chain` is the plain one. */
export const RUNNERS: { id: Runner; label: string }[] = [
  { id: 'chain', label: 'One after another (chain)' },
  { id: 'dispatch', label: 'A lead dispatches to the team (dispatch)' },
  { id: 'concurrent', label: 'All at once (concurrent)' },
  { id: 'external', label: 'By an external system' },
];

/** A runner in a few words, after a team's size: "a lead dispatches". */
export const RUNNER_WORDS: Record<Runner, string> = {
  chain: 'one after another',
  dispatch: 'a lead dispatches',
  concurrent: 'all at once',
  external: 'an external system',
};
