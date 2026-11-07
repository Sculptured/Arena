import type { ColumnDef } from '@tanstack/react-table';
import { MoreHorizontal, Play } from 'lucide-react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { toast } from 'sonner';
import { deleteRun, runs as fetchRuns } from '../api';
import { ApproximateMark, DISK_CHANGED, StatusChip, ago, formatTime } from '../components';
import { Button } from '../components/ui/button';
import { DataTable } from '../components/ui/data-table';
import { Dialog, DialogActions, DialogContent } from '../components/ui/dialog';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '../components/ui/dropdown-menu';
import { FilterChips } from '../components/ui/filter-chips';
import { Input } from '../components/ui/input';
import { PageBody, PageHeader } from '../components/ui/page';
import { Panel } from '../components/ui/panel';
import { EmptyState, ErrorState } from '../components/ui/state';
import NewRunDialog from '../runs/NewRunDialog';
import { type BranchRow, type RowFilter, STATUS_FILTERS, branchRows, rowMatches, runIsRunning, runTitle, statusCounts } from '../runs/runsTable';
import type { RunSummary } from '../types';

export { branchTree } from '../runs/tree';

/** The runs, polled every 3 s while any of them runs. */
function useRuns() {
  const [runs, setRuns] = useState<RunSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const refresh = useCallback(() => {
    fetchRuns().then((r) => { setRuns(r); setError(null); }).catch((e) => setError(String(e.message ?? e)));
  }, []);
  useEffect(() => { refresh(); }, [refresh]);
  const anyRunning = useMemo(() => !!runs?.some(runIsRunning), [runs]);
  useEffect(() => {
    if (!anyRunning) return;
    const t = setInterval(refresh, 3000);
    return () => clearInterval(t);
  }, [anyRunning, refresh]);
  return { runs, error, refresh };
}

function RowMenu({ row, onDelete }: { row: BranchRow; onDelete: (r: RunSummary) => void }) {
  const nav = useNavigate();
  const running = runIsRunning(row.run);
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon" aria-label="Run actions" onClick={(e) => e.stopPropagation()}><MoreHorizontal /></Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent onClick={(e) => e.stopPropagation()}>
        <DropdownMenuItem onSelect={() => nav(`/runs/${row.run.id}/${row.branch.id}`)}>Open timeline</DropdownMenuItem>
        <DropdownMenuItem onSelect={() => nav(`/control/${row.run.id}/${row.branch.id}`)}>Open process view</DropdownMenuItem>
        <DropdownMenuItem disabled={running} className="text-alarm" onSelect={() => onDelete(row.run)}>
          {running ? 'Delete run (a branch is still running)' : 'Delete run…'}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function RunCell({ row }: { row: BranchRow }) {
  if (row.depth === 0) return <span className="block truncate font-medium" title={runTitle(row.run)}>{runTitle(row.run)}</span>;
  return (
    <span className="flex min-w-0 items-center gap-1.5 text-muted" style={{ paddingLeft: (row.depth - 1) * 14 }}>
      <span aria-hidden="true" className="font-mono">↳</span><span>fork</span>
      {row.branch.fork_note && <span className="max-w-[40ch] truncate" title={row.branch.fork_note}>{row.branch.fork_note}</span>}
    </span>
  );
}

function columns(onDelete: (r: RunSummary) => void): ColumnDef<BranchRow, unknown>[] {
  return [
    { id: 'status', header: 'Status', accessorFn: (r) => r.branch.status, cell: ({ row }) => <StatusChip status={row.original.branch.status} />, meta: { className: 'w-28' } },
    { id: 'run', header: 'Run', enableSorting: false, cell: ({ row }) => <RunCell row={row.original} />, meta: { className: 'w-full max-w-0' } },
    { id: 'task', header: 'Task', accessorFn: (r) => r.run.task_id, cell: ({ getValue }) => <span className="font-mono text-xs">{String(getValue())}</span> },
    {
      id: 'branch', header: 'Branch', enableSorting: false, meta: { className: 'hidden md:table-cell' },
      cell: ({ row }) => <span className="flex items-center gap-1.5"><span className="font-mono text-xs text-muted">{row.original.branch.id.slice(0, 8)}</span><ApproximateMark of={row.original.branch} /></span>,
    },
    {
      id: 'started', header: 'Started', accessorFn: (r) => r.branch.created_at, meta: { className: 'w-28 text-right' },
      cell: ({ row }) => <span className="text-xs text-muted" title={formatTime(row.original.branch.created_at)}>{ago(row.original.branch.created_at)}</span>,
    },
    { id: 'menu', header: '', enableSorting: false, cell: ({ row }) => <RowMenu row={row.original} onDelete={onDelete} />, meta: { className: 'w-10 px-1' } },
  ];
}

/** Two-step delete: the run and every branch, freeing its saves. */
function DeleteRunDialog({ run, onClose, onDone }: { run: RunSummary | null; onClose: () => void; onDone: () => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const remove = async () => {
    if (!run) return;
    setBusy(true); setError(null);
    try {
      const r = await deleteRun(run.id);
      window.dispatchEvent(new Event(DISK_CHANGED));
      toast(`Deleted ${runTitle(run)}`, { description: `${r.dropped} saved world${r.dropped === 1 ? '' : 's'} dropped.` });
      onDone(); onClose();
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    setBusy(false);
  };
  return (
    <Dialog open={!!run} onOpenChange={(o) => { if (!o) { setError(null); onClose(); } }}>
      <DialogContent title="Delete this run?" description={run ? `${runTitle(run)} and all ${run.branches.length} of its branches, with their saves. This can't be undone.` : ''}>
        {error && <ErrorState error={error} hint="Nothing was deleted." className="m-4" />}
        <DialogActions onCancel={onClose} onGo={remove} disabled={busy} danger label={busy ? 'Deleting…' : 'Delete run'} />
      </DialogContent>
    </Dialog>
  );
}

function Toolbar({ filter, setFilter, counts }: { filter: RowFilter; setFilter: (f: RowFilter) => void; counts: ReturnType<typeof statusCounts> }) {
  return (
    <div className="flex flex-wrap items-center gap-2 border-b border-line px-3 py-2">
      <FilterChips label="Status" options={STATUS_FILTERS} value={filter.status} counts={counts} onChange={(status) => setFilter({ ...filter, status })} />
      <Input className="ml-auto w-full sm:w-64" placeholder="Filter by name, task or id" value={filter.text}
        onChange={(e) => setFilter({ ...filter, text: e.target.value })} aria-label="Filter runs" />
    </div>
  );
}

export default function Runs() {
  const nav = useNavigate();
  const [sp, setSp] = useSearchParams();
  const { runs, error, refresh } = useRuns();
  const [filter, setFilter] = useState<RowFilter>({ status: 'all', text: '' });
  const [deleting, setDeleting] = useState<RunSummary | null>(null);
  const rows = useMemo(() => (runs ? branchRows(runs) : null), [runs]);
  const cols = useMemo(() => columns(setDeleting), []);
  const newRun = sp.get('new') === 'run';
  const setNewRun = (o: boolean) => setSp(o ? { new: 'run' } : {}, { replace: true });
  return (
    <>
      <PageHeader title="Runs" meta={runs ? `${runs.length} run${runs.length === 1 ? '' : 's'} · every step recorded and saved` : null}
        actions={<Button variant="primary" onClick={() => setNewRun(true)}><Play />New run</Button>} />
      <PageBody>
        {error && runs && <ErrorState error={error} onRetry={refresh} />}
        <Panel>
          <Toolbar filter={filter} setFilter={setFilter} counts={statusCounts(rows ?? [])} />
          <DataTable label="Runs" columns={cols} data={rows} getRowId={(r) => r.branch.id} loadingLabel="Loading runs…"
            error={error} onRetry={refresh} filter={filter} filterFn={(r, f) => rowMatches(r, f as RowFilter)}
            onOpen={(r) => nav(`/runs/${r.run.id}/${r.branch.id}`)} noMatch="No run matches the filter."
            empty={<EmptyState title="No runs yet" hint="Start a run of a task with your team. Every step is recorded, and you can change and rerun from any of them."
              action={<Button variant="primary" onClick={() => setNewRun(true)}><Play />New run</Button>} />} />
        </Panel>
      </PageBody>
      <NewRunDialog open={newRun} onOpenChange={setNewRun} />
      <DeleteRunDialog run={deleting} onClose={() => setDeleting(null)} onDone={refresh} />
    </>
  );
}
