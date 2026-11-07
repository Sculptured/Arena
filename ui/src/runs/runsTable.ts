// The history as table rows: every branch of every run, forks under their parent, and the text
// filter the page applies.
import type { Branch, RunSummary } from '../types';
import { branchTree } from './tree';

export interface BranchRow {
  run: RunSummary;
  branch: Branch;
  /** 0 for a run's first branch; forks are deeper. */
  depth: number;
}

/** Newest run first; each run's branches in tree order. */
export function branchRows(runs: RunSummary[]): BranchRow[] {
  const newest = [...runs].sort((a, b) => b.created_at.localeCompare(a.created_at));
  return newest.flatMap((run) => branchTree(run.branches).map(({ branch, depth }) => ({ run, branch, depth })));
}

/** True when every word of `text` appears in the row's name, team, result, task, ids or note (`name`: the run's name in words). */
export function rowMatches(r: BranchRow, text: string, name = runTitle(r.run)): boolean {
  const hay = [name, r.branch.team.name, r.run.task_id, r.run.id, r.branch.id, r.branch.fork_note ?? '', r.branch.status].join(' ').toLowerCase();
  return text.toLowerCase().split(/\s+/).filter(Boolean).every((w) => hay.includes(w));
}

/** True while some branch of the run is running (the run can't be deleted then). */
export const runIsRunning = (run: RunSummary) => run.branches.some((b) => b.status === 'running');

/** A run's name, else its task's (`taskName`: the task in words, when known), else the task id. */
export const runTitle = (run: RunSummary, taskName?: (id: string) => string) => run.name ?? (taskName?.(run.task_id) || run.task_id);
