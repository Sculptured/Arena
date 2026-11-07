// The suites list: the status filter, the text filter, and the two suites picked to compare.
import type { ListedSuite, SuiteStatus } from './types';

export type SuiteFilter = 'all' | SuiteStatus;

export const SUITE_FILTERS: { id: SuiteFilter; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'running', label: 'Running' },
  { id: 'finished', label: 'Finished' },
  { id: 'stopped', label: 'Stopped' },
];

export function suiteCounts(list: ListedSuite[]): Record<SuiteFilter, number> {
  const out: Record<SuiteFilter, number> = { all: list.length, running: 0, finished: 0, stopped: 0 };
  for (const s of list) out[s.status] += 1;
  return out;
}

export interface ListFilter {
  status: SuiteFilter;
  text: string;
}

export function suiteMatches(s: ListedSuite, f: ListFilter): boolean {
  if (f.status !== 'all' && s.status !== f.status) return false;
  const hay = [s.name, s.team.name, s.id, s.benchmark, ...s.tasks].join(' ').toLowerCase();
  return f.text.toLowerCase().split(/\s+/).filter(Boolean).every((w) => hay.includes(w));
}

/** Picking a suite toggles it; a third pick drops the oldest, so at most two stay picked. */
export function togglePick(picked: string[], id: string): string[] {
  return picked.includes(id) ? picked.filter((x) => x !== id) : [...picked.slice(-1), id];
}

/** "3/10", the share of a suite's runs that ended. */
export const progress = (s: ListedSuite) => (s.runs ? s.ended / s.runs : 0);
