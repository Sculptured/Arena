// A state as a glyph and a word; `abnormal` is the only thing that adds colour (ISA-101).
import type { SuiteStatus } from '../control/types';
import type { BranchStatus } from '../types';

export interface Mark {
  glyph: string;
  word: string;
  abnormal: boolean;
}

const RUN: Record<BranchStatus, Mark> = {
  running: { glyph: '◐', word: 'Running', abnormal: false },
  passed: { glyph: '✓', word: 'Passed', abnormal: false },
  failed: { glyph: '✕', word: 'Failed', abnormal: true },
  infra_error: { glyph: '⚠', word: 'Infra error', abnormal: true },
  interrupted: { glyph: '■', word: 'Interrupted', abnormal: true },
  stopped: { glyph: '■', word: 'Stopped', abnormal: false },
};

/** A run's status. */
export const runMark = (status: BranchStatus): Mark => RUN[status] ?? { glyph: '?', word: status, abnormal: false };

const EXPERIMENT: Record<SuiteStatus, Mark> = {
  running: { glyph: '◐', word: 'Running', abnormal: false },
  finished: { glyph: '■', word: 'Finished', abnormal: false },
  stopped: { glyph: '■', word: 'Stopped', abnormal: false },
};

/** An experiment's status (a stopped one ended early, by choice or because arenad stopped). */
export const experimentMark = (s: SuiteStatus): Mark => EXPERIMENT[s];
