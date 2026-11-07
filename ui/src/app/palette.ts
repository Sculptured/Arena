// The command palette's model: what it lists in each mode and what picking an item does.
import { branchTree } from '../runs/tree';
import type { ListedSuite } from '../control/types';
import type { BranchStatus, RunSummary } from '../types';

export type Mode =
  | { kind: 'root' }
  | { kind: 'compare-runs'; first: string | null }
  | { kind: 'compare-suites'; first: string | null }
  | { kind: 'world' };

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
    { id: 'go-run', group: 'Go to', label: 'Run', keywords: ['home', 'start'], action: { go: '/' } },
    { id: 'go-history', group: 'Go to', label: 'History', keywords: ['runs', 'list'], action: { go: '/history' } },
    { id: 'go-suites', group: 'Go to', label: 'Suites', keywords: ['tests'], action: { go: '/suites' } },
    { id: 'go-team', group: 'Go to', label: 'Team editor', keywords: ['agents', 'edit'], action: { go: '/team' } },
  ];
}

function actions(): Item[] {
  return [
    { id: 'new-run', group: 'Actions', label: 'New run', keywords: ['start', 'task'], action: { go: '/' } },
    { id: 'new-suite', group: 'Actions', label: 'New suite…', keywords: ['start', 'test'], action: { go: '/suites?new=suite' } },
    { id: 'world', group: 'Actions', label: 'World view of a run…', keywords: ['twin', 'digital', 'plant', 'scrub'], action: { mode: { kind: 'world' } } },
    { id: 'cmp-runs', group: 'Actions', label: 'Compare two runs…', keywords: ['diff', 'branch'], action: { mode: { kind: 'compare-runs', first: null } } },
    { id: 'cmp-suites', group: 'Actions', label: 'Compare two suites…', keywords: ['diff', 'regression'], action: { mode: { kind: 'compare-suites', first: null } } },
    { id: 'theme', group: 'Actions', label: 'Switch theme (system, light, dark)', keywords: ['dark', 'light'], action: { theme: true } },
  ];
}

/** Every branch of every run, newest run first, each opening its run page. */
export function runItems(runs: RunSummary[], group = 'Runs', pick?: (branchId: string, runId: string) => Action, only?: BranchStatus[]): Item[] {
  return runs.flatMap((r) => branchTree(r.branches)
    .filter(({ branch: b }) => !only || only.includes(b.status))
    .map(({ branch: b, depth }) => ({
      id: `branch-${b.id}`, group,
      label: `${r.name ?? r.task_id}${depth ? ` · fork ${b.id.slice(0, 6)}` : ''}`,
      hint: `${b.status.replace('_', ' ')} · ${b.id.slice(0, 8)}`,
      keywords: [r.task_id, r.id, b.id, b.status],
      action: pick ? pick(b.id, r.id) : { go: `/runs/${r.id}/${b.id}` },
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

export type Here = { runId: string; branchId: string; page: string } | null;

/** The run branch a page is about (the Run screen's live run, run page, process view, world view), from its address. */
export function hereOf(pathname: string, search = ''): Here {
  const m = /^\/(runs|control|world)\/([^/]+)\/([^/]+)/.exec(pathname);
  if (m) return { page: m[1], runId: decodeURIComponent(m[2]), branchId: decodeURIComponent(m[3]) };
  const q = new URLSearchParams(search);
  const [runId, branchId] = [q.get('run'), q.get('branch')];
  return pathname === '/' && runId && branchId ? { page: 'home', runId, branchId } : null;
}

/** The deeper views of the run on screen: its world view, all its details (event log, trace, deployment estimate), its process view. */
export function hereItems(here: Here): Item[] {
  if (!here) return [];
  const at = `${encodeURIComponent(here.runId)}/${encodeURIComponent(here.branchId)}`;
  const item = (page: string, label: string, keywords: string[]): Item[] => (here.page === page ? [] : [{
    id: `${page}-here`, group: 'This run', label, keywords, action: { go: `/${page}/${at}` },
  }]);
  return [
    ...item('world', 'World view (digital twin)', ['twin', 'world', 'plant']),
    ...item('runs', 'All details: event log, trace, deployment estimate', ['details', 'log', 'trace', 'estimate', 'branches']),
    ...item('control', 'Process view', ['process', 'flows', 'equipment']),
  ];
}

/** The items of a mode: everything at the root, or the runs or sides of a comparison to pick from. */
export function itemsFor(mode: Mode, runs: RunSummary[], suites: ListedSuite[], here: Here = null): Item[] {
  if (mode.kind === 'root') return [...hereItems(here), ...pages(), ...actions(), ...runItems(runs), ...suiteItems(suites)];
  if (mode.kind === 'world') return runItems(runs, 'World view of', (id, run) => ({ go: `/world/${run}/${id}` }));
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
  if (mode.kind === 'world') return 'Pick the run whose world to open…';
  const what = mode.kind === 'compare-runs' ? 'run' : 'suite';
  return mode.first ? `Pick the ${what} to compare it with…` : `Pick the first ${what}…`;
}
