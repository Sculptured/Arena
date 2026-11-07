// The command palette's model: what it lists in each mode and what picking an item does.
import { branchTree } from '../runs/tree';
import type { ListedSuite } from '../control/types';
import type { BranchStatus, RunSummary } from '../types';

export type Mode =
  | { kind: 'root' }
  | { kind: 'compare-runs'; first: string | null }
  | { kind: 'compare-suites'; first: string | null };

export type Action = { go: string } | { mode: Mode } | { theme: true };

export interface Item {
  id: string;
  group: string;
  label: string;
  hint?: string;
  /** Extra words the filter matches (ids, task, status). */
  keywords: string[];
  action: Action;
}

const FINISHED: BranchStatus[] = ['passed', 'failed'];

function pages(): Item[] {
  return [
    { id: 'go-runs', group: 'Go to', label: 'Runs', keywords: ['list'], action: { go: '/' } },
    { id: 'go-suites', group: 'Go to', label: 'Suites', keywords: ['tests'], action: { go: '/suites' } },
    { id: 'go-team', group: 'Go to', label: 'Team', keywords: ['agents'], action: { go: '/team' } },
  ];
}

function actions(): Item[] {
  return [
    { id: 'new-run', group: 'Actions', label: 'New run…', keywords: ['start', 'task'], action: { go: '/?new=run' } },
    { id: 'new-suite', group: 'Actions', label: 'New suite…', keywords: ['start', 'test'], action: { go: '/suites?new=suite' } },
    { id: 'cmp-runs', group: 'Actions', label: 'Compare two runs…', keywords: ['diff', 'branch'], action: { mode: { kind: 'compare-runs', first: null } } },
    { id: 'cmp-suites', group: 'Actions', label: 'Compare two suites…', keywords: ['diff', 'regression'], action: { mode: { kind: 'compare-suites', first: null } } },
    { id: 'theme', group: 'Actions', label: 'Switch theme (system, light, dark)', keywords: ['dark', 'light'], action: { theme: true } },
  ];
}

/** Every branch of every run, newest run first, each opening its run page. */
export function runItems(runs: RunSummary[], group = 'Runs', pick?: (branchId: string) => Action, only?: BranchStatus[]): Item[] {
  return runs.flatMap((r) => branchTree(r.branches)
    .filter(({ branch: b }) => !only || only.includes(b.status))
    .map(({ branch: b, depth }) => ({
      id: `branch-${b.id}`, group,
      label: `${r.name ?? r.task_id}${depth ? ` · fork ${b.id.slice(0, 6)}` : ''}`,
      hint: `${b.status.replace('_', ' ')} · ${b.id.slice(0, 8)}`,
      keywords: [r.task_id, r.id, b.id, b.status],
      action: pick ? pick(b.id) : { go: `/runs/${r.id}/${b.id}` },
    })));
}

export function suiteItems(suites: ListedSuite[], group = 'Suites', pick?: (id: string) => Action): Item[] {
  return suites.map((s) => ({
    id: `suite-${s.id}`, group, label: s.name,
    hint: `${s.status} · ${s.passed}/${s.runs} passed`,
    keywords: [s.id, s.team.name, s.status],
    action: pick ? pick(s.id) : { go: `/suites/${s.id}` },
  }));
}

/** The items of a mode: everything at the root, or the sides of a comparison to pick from. */
export function itemsFor(mode: Mode, runs: RunSummary[], suites: ListedSuite[]): Item[] {
  if (mode.kind === 'root') return [...pages(), ...actions(), ...runItems(runs), ...suiteItems(suites)];
  if (mode.kind === 'compare-runs') {
    const first = mode.first;
    const pick = (id: string): Action => (first ? { go: `/compare/${first}/${id}` } : { mode: { kind: 'compare-runs', first: id } });
    return runItems(runs, first ? 'Compare with' : 'First run', pick, FINISHED).filter((i) => i.id !== `branch-${first}`);
  }
  const first = mode.first;
  const pick = (id: string): Action => (first ? { go: `/suites/compare/${first}/${id}` } : { mode: { kind: 'compare-suites', first: id } });
  return suiteItems(suites, first ? 'Compare with' : 'First suite', pick).filter((i) => i.id !== `suite-${first}`);
}

/** The input's placeholder in each mode. */
export function placeholderOf(mode: Mode): string {
  if (mode.kind === 'root') return 'Go to a run, suite or page, or run a command…';
  const what = mode.kind === 'compare-runs' ? 'run' : 'suite';
  return mode.first ? `Pick the ${what} to compare it with…` : `Pick the first ${what}…`;
}
