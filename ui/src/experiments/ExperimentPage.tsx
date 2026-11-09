import { CircleStop } from 'lucide-react';
import { useState } from 'react';
import { useParams, useSearchParams } from 'react-router-dom';
import { Button } from '../components/ui/button';
import { Dialog, DialogActions, DialogContent } from '../components/ui/dialog';
import { PageBody, PageHeader, type Width } from '../components/ui/page';
import { Panel } from '../components/ui/panel';
import { ErrorState, LoadingState } from '../components/ui/state';
import { StatusMark } from '../components/ui/status';
import { experimentMark } from '@/lib/mark';
import { useTaskNames } from '../home/useHome';
import { usePolled } from '@/lib/usePolled';
import { experiment as fetchExperiment, stopExperiment } from './api';
import { columnOf } from './model';
import { Checks, Setups } from './Results';
import { SuiteWhatMatters } from '../review/WhatMatters';
import type { Experiment } from './types';

/** While an experiment runs, it is read again this often. */
const POLL_MS = 2000;

/** The experiments named, read again while any of them runs. */
function useExperiments(ids: string[]) {
  const key = ids.join(',');
  const load = () => Promise.all(key.split(',').filter(Boolean).map(fetchExperiment));
  const { data, error, reload } = usePolled(load, (v) => v.some((e) => e.status === 'running'), POLL_MS, key);
  return { list: data, error, reload };
}

/** "Larchmont batch on hold · ▼ Usual courier loses capacity": the scenario in a line. */
function ScenarioLine({ e }: { e: Experiment }) {
  const names = e.tasks.map(useTaskNames());
  const disruptions = (e.setups[0]?.scenario?.disruptions ?? []).map((d) => d.label).filter(Boolean);
  return (
    <>
      <span className="max-w-3xl truncate" title={names.join(', ')}>{names.join(', ')}</span>
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
      <Button variant="ghost" onClick={() => setOpen(true)} title={error ?? undefined}><CircleStop />Stop</Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent title="Stop this experiment?" description="The run in progress stops now and no more runs start. Runs that ended stay scored.">
          <DialogActions onCancel={() => setOpen(false)} onGo={stop} danger label="Stop" />
        </DialogContent>
      </Dialog>
    </>
  );
}

/** As wide as the setups need: two side by side fit a narrow page, four a wide one. */
const widthFor = (n: number): Width => (n <= 2 ? 'narrow' : n === 3 ? 'default' : 'wide');

const ended = (e: Experiment) => e.setups.reduce((n, s) => n + s.summary.ended, 0);
const total = (e: Experiment) => e.setups.reduce((n, s) => n + s.summary.runs, 0);

function Header({ list, reload, width }: { list: Experiment[]; reload: () => void; width: Width }) {
  const one = list.length === 1 ? list[0] : null;
  const running = one?.status === 'running';
  return (
    <PageHeader width={width} title={one ? one.name : 'Comparison'}
      status={one && <StatusMark mark={experimentMark(one.status)} title={one.error ?? undefined} />}
      meta={<ScenarioLine e={list[0]} />}
      actions={running && one && <><span className="text-xs text-muted tabular-nums">{ended(one)} of {total(one)} runs</span><StopButton id={one.id} onStopped={reload} /></>} />
  );
}

function View({ ids }: { ids: string[] }) {
  const { list, error, reload } = useExperiments(ids);
  if (!list) {
    return <><PageHeader width="default" title={error ? 'Experiment' : <span className="block h-7 w-72 animate-pulse rounded-md bg-sunken" />} /><PageBody width="default">{error ? <ErrorState error={error} onRetry={reload} /> : <Panel><LoadingState label="Loading the results…" rows={7} /></Panel>}</PageBody></>;
  }
  const reports = list.flatMap((e) => e.setups);
  const width = widthFor(reports.length);
  return (
    <>
      <Header list={list} reload={reload} width={width} />
      <PageBody width={width}>
        {error && <ErrorState error={error} hint="Showing the last results read." onRetry={reload} />}
        {list.filter((e) => e.error && e.error !== 'stopped').map((e) => <ErrorState key={e.id} error={e.error!} hint="The experiment stopped early. The runs that ended are scored." />)}
        <Setups cols={reports.map(columnOf)} />
        {reports.length === 2 && list.every((e) => e.status !== 'running') && <SuiteWhatMatters a={reports[0].id} b={reports[1].id} />}
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
