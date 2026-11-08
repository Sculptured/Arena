import { ListTree, Pause, Play, Square, Undo2 } from 'lucide-react';
import { type ReactNode, useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { setPaused, stopBranch } from '../api';
import { Button } from '../components/ui/button';
import { Dialog, DialogActions, DialogContent } from '../components/ui/dialog';
import { Panel } from '../components/ui/panel';
import { SidePanel } from '../components/ui/side-panel';
import { EmptyState, ErrorState } from '../components/ui/state';
import { StatusMark } from '../components/ui/status';
import { buildLanes, spanOf } from '../control/lanes';
import { cn } from '@/lib/cn';
import ActionTimeline from '../run/ActionTimeline';
import { type Flow, clockText, flowMark, flowOf, isLive, pausesOf } from '../run/flow';
import Inspector from '../run/Inspector';
import { type Result, resultOf } from '../run/result';
import { stepSummary } from '../run/stepSummary';
import { summarize } from '../run/timeline';
import { stepOrder } from '../run/timelineLayout';
import { useBranch } from '../run/useBranch';
import { useStepKeys } from '../run/useStepKeys';
import { useStepParam, withStep } from '../run/useStepParam';
import type { BranchDetail, Event } from '../types';
import TwinView, { useWorldView } from '../world/TwinView';
import { rewindSeq } from '../world/scrub';
import { useViews } from '../world/useWorld';
import { taskOf } from '../run/runStats';
import { useNow } from './useHome';
import { LiveRunReview } from '../review/RunReview';

type Main = 'timeline' | 'twin';

/** Timeline or Digital Twin: two quiet words, the one on screen in the foreground. */
function MainSwitch({ main, setMain }: { main: Main; setMain: (m: Main) => void }) {
  const cls = (on: boolean) => cn('px-1 py-1 text-sm', on ? 'font-medium text-fg' : 'text-muted hover:text-fg');
  return (
    <div className="ml-auto flex gap-3" role="group" aria-label="What to show">
      <button type="button" className={cls(main === 'timeline')} aria-pressed={main === 'timeline'} onClick={() => setMain('timeline')}>Timeline</button>
      <button type="button" className={cls(main === 'twin')} aria-pressed={main === 'twin'} onClick={() => setMain('twin')}>Digital Twin</button>
    </div>
  );
}

/**
 * Pause or Resume, Stop, the status and the time so far (the task and team are its tooltip), and
 * the Timeline | Digital Twin switch; once the run has ended, its result takes the status's place.
 */
function Toolbar({ flow, seconds, title, main, setMain, onPause, onStop, busy, result }: {
  flow: Flow; seconds: number | null; title: string; main: Main; setMain: (m: Main) => void;
  onPause: (paused: boolean) => void; onStop: () => void; busy: boolean; result?: ReactNode;
}) {
  const live = isLive(flow);
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
      {live && (flow === 'paused'
        ? <Button variant="primary" onClick={() => onPause(false)} disabled={busy}><Play />Resume</Button>
        : <Button onClick={() => onPause(true)} disabled={busy || flow === 'starting'}><Pause />Pause</Button>)}
      {live && <Button variant="ghost" onClick={onStop} disabled={busy || flow === 'starting'}><Square />Stop</Button>}
      {result ?? <StatusMark mark={flowMark(flow)} className="text-sm" title={title} />}
      {seconds != null && <span className="font-mono text-sm text-muted" title={title}>{clockText(seconds)}</span>}
      <MainSwitch main={main} setMain={setMain} />
    </div>
  );
}

/** Passed or Failed (or Stopped, Error) and one plain sentence on why; Rewind and Details. */
function ResultLine({ r, rewind, details }: { r: Result; rewind: string | null; details: string }) {
  const m = flowMark(r.flow);
  return (
    <div role="status" className="flex min-w-0 flex-1 flex-wrap items-center gap-x-3 gap-y-2">
      <span className={cn('flex items-center gap-1.5 text-base font-semibold', m.abnormal && 'text-alarm')}><span aria-hidden="true" className="font-mono">{m.glyph}</span>{r.title}</span>
      <span className="min-w-0 text-sm">{r.why}</span>
      <span className="flex gap-1.5">
        {rewind && <Button asChild title="Go back to a step, change the team, and run again from there"><Link to={rewind}><Undo2 />Rewind</Link></Button>}
        <Button variant="ghost" asChild title="Everything about this run: event log, trace, branches, figures"><Link to={details}><ListTree />Details</Link></Button>
      </span>
    </div>
  );
}

/** The step in a sentence or two; the raw record behind Details. */
function StepDrawer({ d, event, onClose }: { d: BranchDetail; event: Event; onClose: () => void }) {
  const s = stepSummary(event, d.events, d.team);
  return (
    <SidePanel title={s.title} onClose={onClose} className="w-full rounded-lg border lg:w-[360px] lg:shrink-0">
      <p className={cn('px-3 py-3 text-sm leading-6', s.abnormal && 'font-medium text-alarm')}>{s.text}</p>
      <details className="border-t border-line">
        <summary className="cursor-pointer px-3 py-2 text-xs font-medium text-muted hover:text-fg">Details</summary>
        <Inspector branchId={d.id} event={event} team={d.team} events={d.events} />
      </details>
    </SidePanel>
  );
}

/** The world being restored: calm words, not a blank screen. */
function Restoring({ line }: { line: string }) {
  return (
    <div className="flex flex-col items-center gap-3 px-6 py-24 text-center" title="Every system starts from the same saved state, so each step can be replayed later. This takes a minute or so.">
      <span className="relative block h-1 w-48 overflow-hidden rounded-full bg-sunken"><span className="absolute inset-y-0 w-1/3 animate-slide rounded-full bg-line-strong" /></span>
      <p className="text-base font-medium">{line}</p>
    </div>
  );
}

type SetStep = (next: number | null | ((s: number | null) => number | null)) => void;

function TimelineMain({ d, now, selected, onSelect }: { d: BranchDetail; now: number; selected: number | null; onSelect: SetStep }) {
  const [models, setModels] = useState(true);
  const lanes = useMemo(() => buildLanes(d.events, d.team, d.start_seq), [d]);
  const live = d.status === 'running';
  const pauses = useMemo(() => pausesOf(d.events, now), [d.events, now]);
  const order = useMemo(() => stepOrder(lanes, models), [lanes, models]);
  useStepKeys(order, onSelect);
  const span = spanOf(lanes);
  return (
    <Panel>
      {span ? <ActionTimeline lanes={lanes} team={d.team} span={span} selected={selected} onSelect={onSelect} models={models} onModels={setModels} live={live} pauses={pauses} />
        : <EmptyState title="The world is ready." />}
    </Panel>
  );
}

function TwinMain({ runId, d, scrub }: { runId: string; d: BranchDetail; scrub: boolean }) {
  const { views, error } = useViews(d.id, d);
  const w = useWorldView(runId, d, views);
  if (!views.length) return <Panel><EmptyState title="No view of the world yet." hint={error ? `Couldn't load it: ${error}` : undefined} /></Panel>;
  return <TwinView w={w} views={views} scrub={scrub} />;
}

/** The first step at or after `seq` a rerun can start from (the first agent step when none is chosen). */
const rewindFrom = (events: Event[], seq: number | null) => rewindSeq(events, (seq ?? 0) - 1);

/** Pause, Resume and Stop: one request at a time, its failure said in words. */
function useControls(branchId: string, reload: () => void) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [stopping, setStopping] = useState(false);
  const act = (f: () => Promise<unknown>) => {
    setBusy(true); setError(null);
    f().then(reload, (e) => setError(e instanceof Error ? e.message : String(e))).finally(() => setBusy(false));
  };
  return { busy, error, stopping, setStopping, pause: (p: boolean) => act(() => setPaused(branchId, p)), stop: () => { setStopping(false); act(() => stopBranch(branchId)); } };
}

/** The timeline (with the step drawer beside it) or the twin. */
function MainArea({ runId, d, main, flow, now, selected, onSelect }: {
  runId: string; d: BranchDetail; main: Main; flow: Flow; now: number; selected: number | null; onSelect: SetStep;
}) {
  const event = main === 'timeline' ? d.events.find((e) => e.seq === selected) : undefined;
  return (
    <div className="flex flex-col gap-3 lg:flex-row lg:items-start">
      <div className="min-w-0 flex-1">
        {main === 'timeline' ? <TimelineMain d={d} now={now} selected={selected} onSelect={onSelect} /> : <TwinMain runId={runId} d={d} scrub={!isLive(flow) || flow === 'paused'} />}
      </div>
      {event && <StepDrawer d={d} event={event} onClose={() => onSelect(null)} />}
    </div>
  );
}

/** The result line of an ended run, or null while it runs. */
function endedLine(d: BranchDetail, runId: string, selected: number | null): ReactNode {
  const r = resultOf(d);
  if (!r) return null;
  const seq = rewindFrom(d.events, selected);
  const base = `/runs/${runId}/${d.id}`;
  return <ResultLine r={r} rewind={seq != null ? `${base}?step=${seq}&fork=1` : null} details={withStep(base, selected)} />;
}

/** A run, live: the toolbar, the timeline or the twin filling in as steps happen, the step drawer, and the result when it ends. */
export function LiveRun({ runId, branchId, nameOf }: { runId: string; branchId: string; nameOf: (taskId: string) => string }) {
  const { detail: d, error, reload } = useBranch(branchId, runId);
  const [main, setMain] = useState<Main>('timeline');
  const [selected, setSelected] = useStepParam();
  const c = useControls(branchId, reload);
  const flow = flowOf(d);
  const now = useNow(isLive(flow));
  const title = d ? `${nameOf(taskOf(d.events))} · ${d.team.name}` : '';
  return (
    <div className="flex flex-col gap-3">
      <Toolbar flow={flow} seconds={d ? summarize(d, d.events, now).seconds : null} title={title} main={main} setMain={setMain} busy={c.busy}
        onPause={c.pause} onStop={() => c.setStopping(true)} result={d ? endedLine(d, runId, selected) : null} />
      {error && <ErrorState error={error} onRetry={reload} />}
      {c.error && <ErrorState error={c.error} hint="The run carries on as it was." />}
      {d && <LiveRunReview runId={runId} d={d} onStep={setSelected} />}
      {d && <MainArea runId={runId} d={d} main={main} flow={flow} now={now} selected={selected} onSelect={setSelected} />}
      <Dialog open={c.stopping} onOpenChange={c.setStopping}>
        <DialogContent title="Stop this run?" description="The agents stop where they are. The run ends now and is not scored; its steps so far stay recorded.">
          <DialogActions onCancel={() => c.setStopping(false)} onGo={c.stop} danger label="Stop the run" />
        </DialogContent>
      </Dialog>
    </div>
  );
}

/** Before the run exists: the toolbar says Starting, and the world being restored. */
export function StartingRun({ title, line }: { title: string; line: string }) {
  return (
    <div className="flex flex-col gap-3">
      <Toolbar flow="starting" seconds={null} title={title} main="timeline" setMain={() => {}} onPause={() => {}} onStop={() => {}} busy />
      <Restoring line={line} />
    </div>
  );
}
