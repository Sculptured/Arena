import type { ColumnDef } from '@tanstack/react-table';
import { MoreHorizontal, Play } from 'lucide-react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { toast } from 'sonner';
import { deleteRun, runs as fetchRuns } from '../api';
import { ApproximateMark, DISK_CHANGED, StatusChip, ago, formatTime } from '../components';
import { taskName } from '../home/summary';
import { useTasks } from '../home/useHome';
import { Button } from '../components/ui/button';
import { DataTable } from '../components/ui/data-table';
import { Dialog, DialogActions, DialogContent } from '../components/ui/dialog';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '../components/ui/dropdown-menu';
import { Input } from '../components/ui/input';
import { PageBody, PageHeader } from '../components/ui/page';
import { Panel } from '../components/ui/panel';
import { EmptyState, ErrorState } from '../components/ui/state';
import { type BranchRow, branchRows, rowMatches, runIsRunning, runTitle } from '../runs/runsTable';
import type { RunSummary } from '../types';

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
        <DropdownMenuItem onSelect={() => nav(openOf(row))}>Open</DropdownMenuItem>
        <DropdownMenuItem onSelect={() => nav(`/runs/${row.run.id}/${row.branch.id}`)}>All details</DropdownMenuItem>
        <DropdownMenuItem onSelect={() => nav(`/control/${row.run.id}/${row.branch.id}`)}>Process view</DropdownMenuItem>
        <DropdownMenuItem disabled={running} className="text-alarm" onSelect={() => onDelete(row.run)}>
          {running ? 'Delete run (it is still running)' : 'Delete run…'}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** A row opens on the Run screen: its result and timeline, with everything else one click away. */
const openOf = (r: BranchRow) => `/?run=${encodeURIComponent(r.run.id)}&branch=${encodeURIComponent(r.branch.id)}`;

function NameCell({ row, name }: { row: BranchRow; name: string }) {
  if (row.depth === 0) return <span className="block truncate font-medium" title={name}>{name}</span>;
  return (
    <span className="flex min-w-0 items-center gap-1.5 text-muted" style={{ paddingLeft: (row.depth - 1) * 14 }}>
      <span aria-hidden="true" className="font-mono">↳</span><span>Rerun</span>
      {row.branch.fork_note && <span className="max-w-[40ch] truncate" title={row.branch.fork_note}>{row.branch.fork_note}</span>}
      <ApproximateMark of={row.branch} />
    </span>
  );
}

function columns(nameOf: (r: BranchRow) => string, onDelete: (r: RunSummary) => void): ColumnDef<BranchRow, unknown>[] {
  return [
    { id: 'name', header: 'Name', enableSorting: false, cell: ({ row }) => <NameCell row={row.original} name={nameOf(row.original)} />, meta: { className: 'w-full max-w-0' } },
    { id: 'team', header: 'Team', accessorFn: (r) => r.branch.team.name, cell: ({ getValue }) => <span className="block max-w-56 truncate text-muted">{String(getValue())}</span>, meta: { className: 'hidden md:table-cell' } },
    { id: 'result', header: 'Result', accessorFn: (r) => r.branch.status, cell: ({ row }) => <StatusChip status={row.original.branch.status} />, meta: { className: 'w-32' } },
    {
      id: 'when', header: 'When', accessorFn: (r) => r.branch.created_at, meta: { className: 'w-28 text-right' },
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

export default function Runs() {
  const nav = useNavigate();
  const { runs, error, refresh } = useRuns();
  const { tasks } = useTasks();
  const [text, setText] = useState('');
  const [deleting, setDeleting] = useState<RunSummary | null>(null);
  const rows = useMemo(() => (runs ? branchRows(runs) : null), [runs]);
  const nameOf = useCallback((r: BranchRow) => runTitle(r.run, (id) => { const t = tasks?.find((x) => x.id === id); return t ? taskName(t) : ''; }), [tasks]);
  const cols = useMemo(() => columns(nameOf, setDeleting), [nameOf]);
  const newRun = <Button variant="primary" asChild><Link to="/"><Play />New run</Link></Button>;
  return (
    <>
      <PageHeader title="History" meta={runs ? `${runs.length} run${runs.length === 1 ? '' : 's'}` : null} actions={newRun} />
      <PageBody>
        {error && runs && <ErrorState error={error} onRetry={refresh} />}
        <Panel>
          <div className="flex border-b border-line px-3 py-2">
            <Input className="w-full sm:w-72" placeholder="Filter by name, team or result" value={text} onChange={(e) => setText(e.target.value)} aria-label="Filter runs" />
          </div>
          <DataTable label="Runs" columns={cols} data={rows} getRowId={(r) => r.branch.id} loadingLabel="Loading runs…"
            error={error} onRetry={refresh} filter={text} filterFn={(r, f) => rowMatches(r, f as string, nameOf(r))}
            onOpen={(r) => nav(openOf(r))} noMatch="No run matches the filter."
            empty={<EmptyState title="No runs yet" hint="Pick a task on the Run screen and press Run. Every step is recorded, and you can rewind to any of them." action={newRun} />} />
        </Panel>
      </PageBody>
      <DeleteRunDialog run={deleting} onClose={() => setDeleting(null)} onDone={refresh} />
    </>
  );
}
