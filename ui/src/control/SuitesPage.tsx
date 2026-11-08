import type { ColumnDef } from '@tanstack/react-table';
import { GitCompare, Play } from 'lucide-react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { ago, formatTime } from '../components';
import { Button } from '../components/ui/button';
import { DataTable } from '../components/ui/data-table';
import { Checkbox } from '../components/ui/input';
import { PageBody, PageHeader } from '../components/ui/page';
import { Panel } from '../components/ui/panel';
import { EmptyState, ErrorState } from '../components/ui/state';
import { StatusMark } from '../components/ui/status';
import NewSuiteDialog, { RUNNERS } from '../suites/NewSuiteDialog';
import { suites as fetchSuites } from './api';
import { suiteMark } from './suite';
import { togglePick } from './suiteList';
import type { ListedSuite } from './types';

/** Pick (to compare two), status, name (its team, scenarios, repeats and runner the tooltip), passed and when. */
function columns(picked: string[], pick: (id: string) => void): ColumnDef<ListedSuite, unknown>[] {
  return [
    {
      id: 'pick', header: () => <span className="sr-only">Compare</span>, enableSorting: false, meta: { className: 'w-8 pr-0' },
      cell: ({ row }) => <Checkbox checked={picked.includes(row.original.id)} onClick={(e) => e.stopPropagation()} onChange={() => pick(row.original.id)} aria-label={`Pick ${row.original.name} to compare`} />,
    },
    { id: 'status', header: 'Status', accessorFn: (s) => s.status, cell: ({ row }) => <StatusMark mark={suiteMark(row.original.status)} />, meta: { className: 'w-28' } },
    {
      id: 'name', header: 'Suite', accessorFn: (s) => s.name, meta: { className: 'w-full max-w-0' },
      cell: ({ row: { original: s } }) => <span className="block truncate font-medium" title={`${s.team.name} · ${s.tasks.length} × ${s.repeats} · ${RUNNERS[s.runner]}`}>{s.name}</span>,
    },
    {
      id: 'passed', header: 'Passed', accessorFn: (s) => s.passed, meta: { className: 'w-24 text-right whitespace-nowrap' },
      cell: ({ row: { original: s } }) => <span className="text-xs" title={`${s.ended} of ${s.runs} runs ended`}>{s.passed} of {s.runs}</span>,
    },
    { id: 'created', header: 'When', accessorFn: (s) => s.created_at, cell: ({ row }) => <span className="text-xs text-muted" title={formatTime(row.original.created_at)}>{ago(row.original.created_at)}</span>, meta: { className: 'w-24 text-right' } },
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
      <PageHeader title={<span title="Each suite runs one team over every scenario, repeated, on the same seeded world.">Suites</span>} className="mx-auto w-full max-w-5xl"
        actions={<>{picked.length > 0 && compare}<Button variant="primary" onClick={() => setNewSuite(true)}><Play />New suite</Button></>} />
      <PageBody className="mx-auto w-full max-w-5xl">
        {error && list && <ErrorState error={error} onRetry={load} />}
        <Panel>
          <DataTable label="Suites" columns={cols} data={list} getRowId={(s) => s.id} loadingLabel="Loading suites…" error={error} onRetry={load}
            onOpen={(s) => nav(`/suites/${s.id}`)}
            sorting={[{ id: 'created', desc: true }]}
            empty={<EmptyState title="No suites yet" />} />
        </Panel>
      </PageBody>
      <NewSuiteDialog open={newSuite} onOpenChange={setNewSuite} />
    </>
  );
}
