import { ListTree } from 'lucide-react';
import { Link, useParams } from 'react-router-dom';
import { StatusChip } from '../components';
import { Button } from '../components/ui/button';
import { PageBody, PageHeader } from '../components/ui/page';
import { Panel, PanelHeader } from '../components/ui/panel';
import { EmptyState, ErrorState, LoadingState } from '../components/ui/state';
import { useBranch } from '../run/useBranch';
import { recordedLine } from '../scenario/scenario';
import type { BranchDetail, RunStartPayload } from '../types';
import { drivenLine } from '../world/page';
import TwinView, { useWorldView } from '../world/TwinView';
import type { BranchView } from '../world/types';
import { useViews } from '../world/useWorld';
import { taskOf } from '../run/runStats';

/** The twin's title and the run's status; the task, the disruption and the rest are its tooltip, the run's other views under ⋯. */
function Header({ d, title }: { d: BranchDetail; title: string }) {
  const driven = drivenLine(d.events);
  const scenario = recordedLine((d.events.find((e) => e.kind === 'run_start')?.payload as RunStartPayload | undefined)?.scenario);
  const tip = [`Digital twin · task ${taskOf(d.events)}`, driven, d.fork_note, scenario && `▼ ${scenario}`].filter(Boolean).join('\n');
  return <PageHeader title={<span title={tip}>{title}</span>} status={<StatusChip status={d.status} />} />;
}

function Body({ runId, d, views }: { runId: string; d: BranchDetail; views: BranchView[] }) {
  const w = useWorldView(runId, d, views);
  if (!w.s.view) return null;
  return (
    <>
      <Header d={d} title={w.s.view.title} />
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
          <EmptyState title="This run has no view of its world." hint={viewError ? `Couldn't load the views: ${viewError}` : undefined} />
        </Panel></PageBody></>
    );
  }
  return <Body runId={runId} d={detail} views={views} />;
}
