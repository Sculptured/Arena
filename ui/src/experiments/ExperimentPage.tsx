import { Square } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useParams, useSearchParams } from 'react-router-dom';
import { Button } from '../components/ui/button';
import { Dialog, DialogActions, DialogContent } from '../components/ui/dialog';
import { PageBody, PageHeader } from '../components/ui/page';
import { Panel } from '../components/ui/panel';
import { ErrorState, LoadingState } from '../components/ui/state';
import { StatusMark } from '../components/ui/status';
import { experimentMark } from '@/lib/mark';
import { taskName } from '../home/summary';
import { useTasks } from '../home/useHome';
import { experiment as fetchExperiment, stopExperiment } from './api';
import { columnOf } from './model';
import { Checks, Columns } from './Results';
import { SuiteWhatMatters } from '../review/WhatMatters';
import type { Experiment } from './types';

/** While an experiment runs, it is read again this often. */
const POLL_MS = 2000;

/** The experiments named, read again while any of them runs. */
function useExperiments(ids: string[]) {
  const [list, setList] = useState<Experiment[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);
  const key = ids.join(',');
  useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const load = () => Promise.all(key.split(',').filter(Boolean).map(fetchExperiment)).then((v) => {
      if (!alive) return;
      setList(v); setError(null);
      if (v.some((e) => e.status === 'running')) timer = setTimeout(load, POLL_MS);
    }).catch((e) => alive && setError(String(e.message ?? e)));
    load();
    return () => { alive = false; clearTimeout(timer); };
  }, [key, tick]);
  return { list, error, reload: () => setTick((t) => t + 1) };
}

/** "Larchmont batch on hold · ▼ Usual courier loses capacity": the scenario in a line. */
function ScenarioLine({ e }: { e: Experiment }) {
  const { tasks } = useTasks();
  const names = e.tasks.map((id) => { const t = tasks?.find((x) => x.id === id); return t ? taskName(t) : id; });
  const disruptions = (e.setups[0]?.scenario?.disruptions ?? []).map((d) => d.label).filter(Boolean);
  return (
    <>
      <span>{names.join(', ')}</span>
      {disruptions.map((d) => <span key={d}><span aria-hidden="true" className="text-warn">▼ </span>{d}</span>)}
    </>
  );
}

function StopButton({ id, onStopped }: { id: string; onStopped: () => void }) {
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const stop = () => { setOpen(false); stopExperiment(id).then(onStopped, (e) => setError(String(e.message ?? e))); };
  return (
    <>
      <Button variant="ghost" onClick={() => setOpen(true)} title={error ?? undefined}><Square />Stop</Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent title="Stop this experiment?" description="The run in progress stops now and no more runs start. Runs that ended stay scored.">
          <DialogActions onCancel={() => setOpen(false)} onGo={stop} danger label="Stop" />
        </DialogContent>
      </Dialog>
    </>
  );
}

const ended = (e: Experiment) => e.setups.reduce((n, s) => n + s.summary.ended, 0);
const total = (e: Experiment) => e.setups.reduce((n, s) => n + s.summary.runs, 0);

function Header({ list, reload }: { list: Experiment[]; reload: () => void }) {
  const one = list.length === 1 ? list[0] : null;
  const running = one?.status === 'running';
  return (
    <PageHeader width="wide" title={one ? one.name : 'Comparison'}
      status={one && <StatusMark mark={experimentMark(one.status)} title={one.error ?? undefined} />}
      meta={<ScenarioLine e={list[0]} />}
      actions={running && one && <><span className="font-mono text-xs text-muted">{ended(one)} / {total(one)} runs</span><StopButton id={one.id} onStopped={reload} /></>} />
  );
}

function View({ ids }: { ids: string[] }) {
  const { list, error, reload } = useExperiments(ids);
  if (!list) {
    return <><PageHeader width="wide" title={error ? 'Experiment' : <span className="block h-7 w-72 animate-pulse rounded-md bg-sunken" />} /><PageBody width="wide">{error ? <ErrorState error={error} onRetry={reload} /> : <Panel><LoadingState label="Loading the results…" rows={7} /></Panel>}</PageBody></>;
  }
  const reports = list.flatMap((e) => e.setups);
  return (
    <>
      <Header list={list} reload={reload} />
      <PageBody width="wide">
        {error && <ErrorState error={error} hint="Showing the last results read." onRetry={reload} />}
        {list.filter((e) => e.error && e.error !== 'stopped').map((e) => <ErrorState key={e.id} error={e.error!} hint="The experiment stopped early. The runs that ended are scored." />)}
        {reports.length === 2 && list.every((e) => e.status !== 'running') && <SuiteWhatMatters a={reports[0].id} b={reports[1].id} />}
        <Columns cols={reports.map(columnOf)} />
        <Checks reports={reports} />
      </PageBody>
    </>
  );
}

/** An experiment's results, one column per setup (`/experiments/:id`). */
export default function ExperimentPage() {
  const { id = '' } = useParams();
  return <View ids={[id]} />;
}

/** Several experiments' setups side by side (`/experiments/compare?ids=a,b`). */
export function ComparePage() {
  const [sp] = useSearchParams();
  return <View ids={(sp.get('ids') ?? '').split(',').filter(Boolean)} />;
}
