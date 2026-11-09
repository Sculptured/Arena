import { MoreHorizontal } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { openPalette } from '../app/palette';
import { StatusChip } from '../components';
import { Button } from '../components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuTrigger } from '../components/ui/dropdown-menu';
import { branchTree } from '../runs/tree';
import type { Branch, BranchDetail } from '../types';
import { rewindSeq } from '../world/scrub';
import { withStep } from './useStepParam';

export type RunPage = 'details' | 'world' | 'process' | 'live';

const VIEWS: { page: Exclude<RunPage, 'live'>; label: string; path: string }[] = [
  { page: 'details', label: 'Timeline and details', path: 'runs' },
  { page: 'world', label: 'World view', path: 'world' },
  { page: 'process', label: 'Process view', path: 'control' },
];

/**
 * A run's "⋯": its other views, comparing it, its reruns, and rewinding it. Rewind lives here
 * only: it is for digging into one run, not part of testing agents.
 */
export default function RunMenu({ runId, d, page, step, siblings = [] }: {
  runId: string; d: BranchDetail; page: RunPage; step: number | null; siblings?: Branch[];
}) {
  const nav = useNavigate();
  const branchId = d.id;
  const running = d.status === 'running';
  // The first step at or after the one picked that a rerun can start from (the first agent step when none is).
  const rewindAt = rewindSeq(d.events, (step ?? 0) - 1);
  const at = `${encodeURIComponent(runId)}/${encodeURIComponent(branchId)}`;
  const reruns = siblings.length > 1 ? branchTree(siblings) : [];
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild><Button variant="ghost" size="icon" aria-label="More about this run"><MoreHorizontal /></Button></DropdownMenuTrigger>
      <DropdownMenuContent className="w-64">
        {VIEWS.filter((v) => v.page !== page).map((v) => (
          <DropdownMenuItem key={v.page} onSelect={() => nav(withStep(`/${v.path}/${at}`, step))}>{v.label}</DropdownMenuItem>
        ))}
        <DropdownMenuItem onSelect={() => openPalette({ kind: 'compare-runs', first: branchId })}>Compare with another run…</DropdownMenuItem>
        {reruns.length > 0 && <DropdownMenuLabel>Reruns</DropdownMenuLabel>}
        {reruns.map(({ branch: b, depth }) => (
          <DropdownMenuItem key={b.id} onSelect={() => nav(`/runs/${encodeURIComponent(runId)}/${encodeURIComponent(b.id)}`)} className={b.id === branchId ? 'bg-accent-soft' : ''}>
            <span style={{ paddingLeft: depth * 12 }} className="min-w-0 flex-1 truncate text-xs">{depth ? `↳ ${b.fork_note ?? 'Rerun'}` : 'Original'}</span>
            <StatusChip status={b.status} />
          </DropdownMenuItem>
        ))}
        <DropdownMenuLabel>Advanced</DropdownMenuLabel>
        <DropdownMenuItem disabled={running || rewindAt == null} onSelect={() => rewindAt != null && nav(`/runs/${at}?step=${rewindAt}&fork=1`)}>
          {running ? 'Rewind (when the run ends)' : rewindAt == null ? 'Rewind (no step to rerun from)' : `Rewind to step ${rewindAt} and rerun…`}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
