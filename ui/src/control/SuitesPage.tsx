import type { ColumnDef } from '@tanstack/react-table';
import { GitCompare, Play } from 'lucide-react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { ago, formatTime } from '../components';
import { Button } from '../components/ui/button';
import { DataTable } from '../components/ui/data-table';
import { FilterChips } from '../components/ui/filter-chips';
import { Checkbox, Input } from '../components/ui/input';
import { PageBody, PageHeader } from '../components/ui/page';
import { Panel } from '../components/ui/panel';
import { EmptyState, ErrorState } from '../components/ui/state';
import { StatusMark } from '../components/ui/status';
import NewSuiteDialog, { RUNNERS } from '../suites/NewSuiteDialog';
import { suites as fetchSuites } from './api';
import { suiteMark } from './suite';
import { type ListFilter, SUITE_FILTERS, progress, suiteCounts, suiteMatches, togglePick } from './suiteList';
import type { ListedSuite } from './types';

function Meter({ fraction, label }: { fraction: number; label: string }) {
  return (
    <span className="flex items-center gap-2">
      <span className="font-mono text-xs">{label}</span>
      <span aria-hidden="true" className="relative hidden h-1.5 w-16 overflow-hidden rounded-full bg-sunken ring-1 ring-line sm:block">
        <span className="absolute inset-y-0 left-0 bg-line-strong" style={{ width: `${fraction * 100}%` }} />
      </span>
    </span>
  );
}

function columns(picked: string[], pick: (id: string) => void): ColumnDef<ListedSuite, unknown>[] {
  return [
    {
      id: 'pick', header: () => <span className="sr-only">Compare</span>, enableSorting: false, meta: { className: 'w-8 pr-0' },
      cell: ({ row }) => <Checkbox checked={picked.includes(row.original.id)} onClick={(e) => e.stopPropagation()} onChange={() => pick(row.original.id)} aria-label={`Pick ${row.original.name} to compare`} />,
    },
    { id: 'status', header: 'Status', accessorFn: (s) => s.status, cell: ({ row }) => <StatusMark mark={suiteMark(row.original.status)} />, meta: { className: 'w-28' } },
    { id: 'name', header: 'Suite', accessorFn: (s) => s.name, cell: ({ row }) => <span className="font-medium">{row.original.name}</span> },
    { id: 'team', header: 'Team', accessorFn: (s) => s.team.name, cell: ({ getValue }) => <span className="text-muted">{String(getValue())}</span>, meta: { className: 'hidden lg:table-cell' } },
    { id: 'size', header: 'Scenarios × repeats', enableSorting: false, cell: ({ row }) => <span className="font-mono text-xs">{row.original.tasks.length} × {row.original.repeats}</span>, meta: { className: 'hidden md:table-cell' } },
    { id: 'runner', header: 'Runs made by', enableSorting: false, cell: ({ row }) => <span className="text-xs text-muted">{RUNNERS[row.original.runner]}</span>, meta: { className: 'hidden xl:table-cell' } },
    { id: 'ended', header: 'Ended', accessorFn: progress, cell: ({ row }) => <Meter fraction={progress(row.original)} label={`${row.original.ended}/${row.original.runs}`} /> },
    { id: 'passed', header: 'Passed', accessorFn: (s) => s.passed, cell: ({ row }) => <span className="font-mono text-xs">{row.original.passed}</span>, meta: { className: 'text-right' } },
    { id: 'created', header: 'Created', accessorFn: (s) => s.created_at, cell: ({ row }) => <span className="text-xs text-muted" title={formatTime(row.original.created_at)}>{ago(row.original.created_at)}</span>, meta: { className: 'hidden sm:table-cell text-right' } },
  ];
}

function useSuites() {
  const [list, setList] = useState<ListedSuite[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(() => { fetchSuites().then((l) => { setList(l); setError(null); }).catch((e) => setError(String(e.message ?? e))); }, []);
  useEffect(() => { load(); }, [load]);
  return { list, error, load };
}

export default function SuitesPage() {
  const nav = useNavigate();
  const [sp, setSp] = useSearchParams();
  const { list, error, load } = useSuites();
  const [filter, setFilter] = useState<ListFilter>({ status: 'all', text: '' });
  const [picked, setPicked] = useState<string[]>([]);
  const cols = useMemo(() => columns(picked, (id) => setPicked((p) => togglePick(p, id))), [picked]);
  const newSuite = sp.get('new') === 'suite';
  const setNewSuite = (o: boolean) => setSp(o ? { new: 'suite' } : {}, { replace: true });
  const compare = (
    <Button disabled={picked.length !== 2} onClick={() => nav(`/suites/compare/${picked[0]}/${picked[1]}`)} title={picked.length === 2 ? 'Compare the two picked suites' : 'Pick two suites in the list to compare them'}>
      <GitCompare />Compare {picked.length === 2 ? 'the two' : `(${picked.length}/2 picked)`}
    </Button>
  );
  return (
    <>
      <PageHeader title="Suites" meta="Each suite runs one team over every scenario, repeated, on the same seeded world."
        actions={<>{compare}<Button variant="primary" onClick={() => setNewSuite(true)}><Play />New suite</Button></>} />
      <PageBody>
        {error && list && <ErrorState error={error} onRetry={load} />}
        <Panel>
          <div className="flex flex-wrap items-center gap-2 border-b border-line px-3 py-2">
            <FilterChips label="Status" options={SUITE_FILTERS} value={filter.status} counts={suiteCounts(list ?? [])} onChange={(status) => setFilter({ ...filter, status })} />
            <Input className="ml-auto w-full sm:w-64" placeholder="Filter by name, team or scenario" value={filter.text} onChange={(e) => setFilter({ ...filter, text: e.target.value })} aria-label="Filter suites" />
          </div>
          <DataTable label="Suites" columns={cols} data={list} getRowId={(s) => s.id} loadingLabel="Loading suites…" error={error} onRetry={load}
            filter={filter} filterFn={(s, f) => suiteMatches(s, f as ListFilter)} onOpen={(s) => nav(`/suites/${s.id}`)} noMatch="No suite matches the filter."
            sorting={[{ id: 'created', desc: true }]}
            empty={<EmptyState title="No suites yet" hint="A suite runs your team over each scenario, several times, so you can compare teams fairly."
              action={<Button variant="primary" onClick={() => setNewSuite(true)}><Play />New suite</Button>} />} />
        </Panel>
      </PageBody>
      <NewSuiteDialog open={newSuite} onOpenChange={setNewSuite} />
    </>
  );
}
