// The command palette's model: what it lists in each mode and what picking an item does.
import { branchTree } from '../runs/tree';
import type { ListedExperiment } from '../experiments/types';
import type { BranchStatus, RunSummary } from '../types';

export type Mode =
  | { kind: 'root' }
  | { kind: 'compare-runs'; first: string | null }
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
    { id: 'go-experiments', group: 'Go to', label: 'Experiments', keywords: ['home', 'suites', 'tests'], action: { go: '/' } },
    { id: 'go-runs', group: 'Go to', label: 'Runs', keywords: ['history', 'list'], action: { go: '/runs' } },
    { id: 'go-team', group: 'Go to', label: 'Team editor', keywords: ['agents', 'edit'], action: { go: '/team' } },
  ];
}

function actions(): Item[] {
  return [
    { id: 'new-experiment', group: 'Actions', label: 'New experiment', keywords: ['start', 'compare', 'suite', 'test'], action: { go: '/experiments/new' } },
    { id: 'new-run', group: 'Actions', label: 'Run once', keywords: ['start', 'task', 'single'], action: { go: '/run' } },
    { id: 'world', group: 'Actions', label: 'World view of a run…', keywords: ['twin', 'digital', 'plant', 'scrub'], action: { mode: { kind: 'world' } } },
    { id: 'cmp-runs', group: 'Actions', label: 'Compare two runs…', keywords: ['diff', 'branch'], action: { mode: { kind: 'compare-runs', first: null } } },
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

export function experimentItems(list: ListedExperiment[]): Item[] {
  return list.map((e) => ({
    id: `experiment-${e.id}`, group: 'Experiments', label: e.name,
    hint: `${e.setups.length} setup${e.setups.length === 1 ? '' : 's'} · ${e.status}`,
    keywords: [e.id, e.status, ...e.setups.map((s) => s.name)],
    action: { go: `/experiments/${e.id}` },
  }));
}

export type Here = { runId: string; branchId: string; page: string } | null;

/** The run branch a page is about (the Run screen's live run, run page, process view, world view), from its address. */
export function hereOf(pathname: string, search = ''): Here {
  const m = /^\/(runs|control|world)\/([^/]+)\/([^/]+)/.exec(pathname);
  if (m) return { page: m[1], runId: decodeURIComponent(m[2]), branchId: decodeURIComponent(m[3]) };
  const q = new URLSearchParams(search);
  const [runId, branchId] = [q.get('run'), q.get('branch')];
  return pathname === '/run' && runId && branchId ? { page: 'home', runId, branchId } : null;
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
    ...item('runs', 'All details', ['details', 'log', 'trace', 'estimate', 'branches']),
    ...item('control', 'Process view', ['process', 'flows', 'equipment']),
  ];
}

/** The items of a mode: everything at the root, or the runs or sides of a comparison to pick from. */
export function itemsFor(mode: Mode, runs: RunSummary[], experiments: ListedExperiment[], here: Here = null): Item[] {
  if (mode.kind === 'root') return [...hereItems(here), ...pages(), ...actions(), ...experimentItems(experiments), ...runItems(runs)];
  if (mode.kind === 'world') return runItems(runs, 'World view of', (id, run) => ({ go: `/world/${run}/${id}` }));
  const first = mode.first;
  const pick = (id: string): Action => (first ? { go: `/compare/${first}/${id}` } : { mode: { kind: 'compare-runs', first: id } });
  return runItems(runs, first ? 'Compare with' : 'First run', pick, FINISHED).filter((i) => i.id !== `branch-${first}`);
}

/** The input's placeholder in each mode. */
export function placeholderOf(mode: Mode): string {
  if (mode.kind === 'root') return 'Go to an experiment, a run or a page…';
  if (mode.kind === 'world') return 'Pick the run whose world to open…';
  return mode.first ? 'Pick the run to compare it with…' : 'Pick the first run…';
}

/** Asks the app to open the palette in `mode` (a run's ⋯ → "Compare with another run…"). */
export const PALETTE_EVENT = 'arena:palette';

export function openPalette(mode: Mode) {
  window.dispatchEvent(new CustomEvent<Mode>(PALETTE_EVENT, { detail: mode }));
}
