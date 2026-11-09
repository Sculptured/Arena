import { FlaskConical, GitCompare, Plus } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { ago, formatTime } from '../components';
import { Button } from '../components/ui/button';
import { Checkbox } from '../components/ui/input';
import { PageBody, PageHeader } from '../components/ui/page';
import { Panel } from '../components/ui/panel';
import { ErrorState, LoadingState } from '../components/ui/state';
import { StatusMark } from '../components/ui/status';
import { experimentMark } from '@/lib/mark';
import { taskName } from '../home/summary';
import { useTasks } from '../home/useHome';
import { cn } from '@/lib/cn';
import { experiments as fetchExperiments } from './api';
import type { ListedExperiment, SetupLine } from './types';

/** While any experiment runs, the list is read again this often. */
const POLL_MS = 3000;

function useList() {
  const [list, setList] = useState<ListedExperiment[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(() => { fetchExperiments().then((l) => { setList(l); setError(null); }).catch((e) => setError(String(e.message ?? e))); }, []);
  useEffect(() => { load(); }, [load]);
  const running = !!list?.some((e) => e.status === 'running');
  useEffect(() => {
    if (!running) return;
    const t = setInterval(load, POLL_MS);
    return () => clearInterval(t);
  }, [running, load]);
  return { list, error, load };
}

/** A setup in a row: its name and a bar of how many of its runs passed, out of all it asked for. */
function SetupBar({ s, label }: { s: SetupLine; label: string }) {
  const w = (n: number) => `${s.runs ? (n / s.runs) * 100 : 0}%`;
  return (
    <div className="flex min-w-0 items-center gap-2" title={`${s.name} · ${s.passed} of ${s.ended} ended runs passed (${s.runs} asked)`}>
      <span className="w-32 truncate text-xs text-muted">{label}</span>
      <span className="relative h-1.5 w-20 shrink-0 overflow-hidden rounded-full bg-sunken">
        <span className="absolute inset-y-0 left-0 bg-line-strong/60" style={{ width: w(s.ended) }} />
        <span className="absolute inset-y-0 left-0 bg-ok" style={{ width: w(s.passed) }} />
      </span>
      <span className="w-9 text-xs text-muted tabular-nums">{s.passed}/{s.runs}</span>
    </div>
  );
}

function ExperimentRow({ e, picked, onPick, picking, task }: { e: ListedExperiment; picked: boolean; onPick: () => void; picking: boolean; task: string }) {
  return (
    <li className="group flex items-center gap-3 border-t border-line px-4 py-3 first:border-t-0 hover:bg-hover">
      <Checkbox checked={picked} onChange={onPick} aria-label={`Pick ${e.name} to compare`}
        className={cn('transition-opacity', !picking && 'opacity-0 group-hover:opacity-100 focus-visible:opacity-100')} />
      <Link to={`/experiments/${e.id}`} className="flex min-w-0 flex-1 flex-col gap-0.5 sm:flex-row sm:items-center sm:gap-6">
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-sm font-medium">{e.name}</span>
          <span className="truncate text-xs text-muted">{task}</span>
        </span>
        <span className="hidden flex-col gap-1 md:flex">{e.setups.map((s) => <SetupBar key={s.suite_id} s={s} label={s.name === e.name ? s.team_name : s.name} />)}</span>
        <span className="flex w-36 items-center justify-end gap-3">
          {e.status !== 'finished' && <StatusMark mark={experimentMark(e.status)} />}
          <span className="text-xs text-faint" title={formatTime(e.created_at)}>{ago(e.created_at)}</span>
        </span>
      </Link>
    </li>
  );
}

const newButton = <Button variant="primary" asChild><Link to="/experiments/new"><Plus />New experiment</Link></Button>;

/** Nothing run yet: what an experiment is, in one line, and the one thing to do. */
function Empty() {
  return (
    <div className="flex flex-col items-center gap-4 px-6 py-20 text-center">
      <span className="grid size-12 place-items-center rounded-xl bg-sunken text-muted"><FlaskConical className="size-6" /></span>
      <div className="flex flex-col gap-1">
        <p className="text-base font-semibold">Compare agent setups on the same scenario</p>
        <p className="text-sm text-muted">Each setup runs several times; you see pass rate, safety and cost side by side.</p>
      </div>
      {newButton}
    </div>
  );
}

/** The experiments, newest first; pick two or more to see their setups side by side. */
export default function ExperimentsPage() {
  const nav = useNavigate();
  const { list, error, load } = useList();
  const { tasks } = useTasks();
  const [picked, setPicked] = useState<string[]>([]);
  const toggle = (id: string) => setPicked((p) => (p.includes(id) ? p.filter((x) => x !== id) : [...p, id].slice(-4)));
  const taskOf = (e: ListedExperiment) => e.tasks.map((id) => { const t = tasks?.find((x) => x.id === id); return t ? taskName(t) : id; }).join(', ');
  const compare = picked.length >= 2 && (
    <Button onClick={() => nav(`/experiments/compare?ids=${picked.map(encodeURIComponent).join(',')}`)}><GitCompare />Compare {picked.length}</Button>
  );
  return (
    <>
      <PageHeader width="default" title="Experiments" actions={list?.length ? <>{compare}{newButton}</> : null} />
      <PageBody width="default">
        {error && <ErrorState error={error} onRetry={load} />}
        {!list && !error && <Panel><LoadingState label="Loading experiments…" rows={4} /></Panel>}
        {list && !list.length && <Panel><Empty /></Panel>}
        {list && list.length > 0 && (
          <Panel>
            <ul>{list.map((e) => <ExperimentRow key={e.id} e={e} task={taskOf(e)} picked={picked.includes(e.id)} picking={picked.length > 0} onPick={() => toggle(e.id)} />)}</ul>
          </Panel>
        )}
      </PageBody>
    </>
  );
}
