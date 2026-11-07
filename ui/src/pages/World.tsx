import { ListTree, Workflow } from 'lucide-react';
import { Link, useParams } from 'react-router-dom';
import { StatusChip } from '../components';
import { Button } from '../components/ui/button';
import { PageBody, PageHeader } from '../components/ui/page';
import { Panel, PanelHeader } from '../components/ui/panel';
import { EmptyState, ErrorState, LoadingState } from '../components/ui/state';
import { Tag } from '../components/ui/status';
import { useBranch } from '../run/useBranch';
import { withStep } from '../run/useStepParam';
import { recordedLine } from '../scenario/scenario';
import type { BranchDetail, RunStartPayload } from '../types';
import { drivenLine } from '../world/page';
import TwinView, { useWorldView } from '../world/TwinView';
import type { BranchView } from '../world/types';
import { useViews } from '../world/useWorld';
import { taskOf } from '../run/runStats';

function Header({ runId, d, title, step }: { runId: string; d: BranchDetail; title: string; step: number | null }) {
  const driven = drivenLine(d.events);
  const scenario = recordedLine((d.events.find((e) => e.kind === 'run_start')?.payload as RunStartPayload | undefined)?.scenario);
  return (
    <PageHeader title={title}
      status={<><StatusChip status={d.status} />{driven && <Tag>{driven}</Tag>}<span className="font-mono text-xs text-faint">{d.id.slice(0, 8)}</span></>}
      actions={<>
        <Button size="sm" asChild><Link to={withStep(`/runs/${runId}/${d.id}`, step)}><ListTree />Run page</Link></Button>
        <Button size="sm" asChild><Link to={withStep(`/control/${runId}/${d.id}`, step)}><Workflow />Process view</Link></Button>
      </>}
      meta={<>
        <span>Digital twin · task <span className="font-mono">{taskOf(d.events)}</span></span>
        {d.fork_note && <span>{d.fork_note}</span>}
        {scenario && <span className="text-warn">▼ {scenario}</span>}
      </>} />
  );
}

function Body({ runId, d, views }: { runId: string; d: BranchDetail; views: BranchView[] }) {
  const w = useWorldView(runId, d, views);
  if (!w.s.view) return null;
  return (
    <>
      <Header runId={runId} d={d} title={w.s.view.title} step={w.step} />
      <PageBody className="gap-2.5"><TwinView w={w} views={views} /></PageBody>
    </>
  );
}

/** The digital twin of the run's world at any step: an HMI overview, a scrubber, and Rewind here. */
export default function World() {
  const { runId = '', branchId = '' } = useParams();
  const { detail, error, reload } = useBranch(branchId, runId);
  const { views, error: viewError } = useViews(branchId, detail);
  if (!detail) {
    return <><PageHeader title="World" /><PageBody>{error ? <ErrorState error={error} onRetry={reload} /> : <Panel><LoadingState label="Loading the world…" rows={6} /></Panel>}</PageBody></>;
  }
  if (!views.length) {
    return (
      <><PageHeader title="World" status={<StatusChip status={detail.status} />} actions={<Button size="sm" asChild><Link to={`/runs/${runId}/${branchId}`}><ListTree />Run page</Link></Button>} />
        <PageBody><Panel><PanelHeader title="Digital twin" />
          <EmptyState title="This run has no view of its world." hint={viewError ? `Couldn't load the views: ${viewError}` : 'Its benchmark draws no digital twin (no `view` method), or the run recorded none yet. The run page and the process view show every step.'} />
        </Panel></PageBody></>
    );
  }
  return <Body runId={runId} d={detail} views={views} />;
}
