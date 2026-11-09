import { useMemo, useState } from 'react';
import { useParams } from 'react-router-dom';
import { StatusChip } from '../components';
import { PageBody, PageHeader } from '../components/ui/page';
import { Panel, PanelHeader } from '../components/ui/panel';
import { Split } from '../components/ui/split';
import { ErrorState, LoadingState } from '../components/ui/state';
import { taskOf } from '../run/runStats';
import ActionTimeline from '../run/ActionTimeline';
import Inspector from '../run/Inspector';
import { stepOrder } from '../run/timelineLayout';
import { useBranch } from '../run/useBranch';
import { useStepKeys } from '../run/useStepKeys';
import { useStepParam } from '../run/useStepParam';
import RunMenu from '../run/RunMenu';
import type { BranchDetail } from '../types';
import { actionsOf, worldInfo } from './actions';
import { buildLanes, spanOf } from './lanes';
import { processView } from './process';
import ProcessDiagram from './ProcessDiagram';
import TrendsPanel from './TrendsPanel';

/** The selected step, in the address (`?step=`) the run page and the world view share. */
function useSelection(order: number[]) {
  const [selected, setSelected] = useStepParam();
  useStepKeys(order, setSelected);
  return [selected, setSelected] as const;
}

function Main({ d, selected, onSelect, models, setModels }: { d: BranchDetail; selected: number | null; onSelect: (s: number) => void; models: boolean; setModels: (m: boolean) => void }) {
  const actions = useMemo(() => actionsOf(d.events, d.start_seq), [d]);
  const lanes = useMemo(() => buildLanes(d.events, d.team, d.start_seq), [d]);
  const view = useMemo(() => processView(d.events, actions, d.team), [d, actions]);
  const running = d.status === 'running';
  return (
    <div className="flex min-w-0 flex-col gap-3">
      <Panel><PanelHeader title="Process" meta="Agents as units, systems as equipment; heavy lines write, dashed lines read" /><ProcessDiagram view={view} /></Panel>
      <Panel><PanelHeader title={`Actions${running ? ' · live' : ''}`} />
        <ActionTimeline lanes={lanes} team={d.team} span={spanOf(lanes)} selected={selected} onSelect={onSelect} models={models} onModels={setModels} />
      </Panel>
      <Panel><PanelHeader title="Trends" /><TrendsPanel events={d.events} actions={actions} /></Panel>
    </div>
  );
}

/** The run as a process (L2), its action timeline and trends, and one step's trace (L4). Live while it runs. */
export default function ProcessPage() {
  const { runId = '', branchId = '' } = useParams();
  const { detail: d, error, reload } = useBranch(branchId);
  const [models, setModels] = useState(true);
  const order = useMemo(() => (d ? stepOrder(buildLanes(d.events, d.team, d.start_seq), models) : []), [d, models]);
  const [selected, setSelected] = useSelection(order);
  if (!d) return <><PageHeader title="Process" /><PageBody>{error ? <ErrorState error={error} onRetry={reload} /> : <Panel><LoadingState label="Loading the run…" rows={5} /></Panel>}</PageBody></>;
  const task = taskOf(d.events);
  const info = worldInfo(d.events);
  return (
    <>
      <PageHeader title={task ? `Process · ${task}` : 'Process'} status={<StatusChip status={d.status} />}
        meta={!info.known && 'Recorded before Arena kept each tool\'s system and effect: those are shown as unknown.'}
        actions={<RunMenu runId={runId} d={d} page="process" step={selected} />} />
      <PageBody>
        {error && <ErrorState error={error} onRetry={reload} />}
        {d.error && <ErrorState error={d.error} hint="The run stopped with this error. Its steps up to here are recorded." />}
        <Split storageKey="arena.process.side" main={<Main d={d} selected={selected} onSelect={setSelected} models={models} setModels={setModels} />}
          side={<Panel className="lg:sticky lg:top-16 lg:max-h-[calc(100vh-88px)] lg:overflow-auto"><PanelHeader title="Step" /><Inspector branchId={d.id} event={d.events.find((e) => e.seq === selected) ?? null} team={d.team} events={d.events} /></Panel>} />
      </PageBody>
    </>
  );
}
