import { ChevronDown, Factory, GitBranch, GitCompare, Workflow } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { Link, useLocation, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { ApproximateMark, StatusChip } from '../components';
import { Button } from '../components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuTrigger } from '../components/ui/dropdown-menu';
import { PageBody, PageHeader, Stat } from '../components/ui/page';
import { Panel, PanelHeader } from '../components/ui/panel';
import { Split } from '../components/ui/split';
import { ErrorState, LoadingState, Notice } from '../components/ui/state';
import { StatusMark } from '../components/ui/status';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../components/ui/tabs';
import { buildLanes, spanOf } from '../control/lanes';
import { costIsUnknown, teamProviders } from '../providers';
import { worldLine } from '../restore';
import ActionTimeline from '../run/ActionTimeline';
import EventLog from '../run/EventLog';
import ForkPanel from '../run/ForkPanel';
import Inspector from '../run/Inspector';
import MetricsPanel from '../run/Metrics';
import { pausesOf } from '../run/flow';
import { checkMark, runStats, safetyLine, taskOf } from '../run/runStats';
import { formatCost, summarize } from '../run/timeline';
import { stepOrder } from '../run/timelineLayout';
import { useBranch } from '../run/useBranch';
import { useStepKeys } from '../run/useStepKeys';
import { useStepParam, withStep } from '../run/useStepParam';
import { type ForkChoice, forkChoiceOf } from '../run/architecture';
import { recordedLine } from '../scenario/scenario';
import { drivenLine } from '../world/page';
import { Tag } from '../components/ui/status';
import Waterfall from '../run/Waterfall';
import { branchTree } from '../runs/tree';
import type { Branch, BranchDetail, Event, RunStartPayload } from '../types';

/** How the run's agents ran, from its `run_start` (old runs: a chain with a handoff required). */
export function runnerLine(events: Event[]): string {
  const rs = events.find((e) => e.kind === 'run_start')?.payload as RunStartPayload | undefined;
  if (!rs) return '';
  if (!rs.runner) return 'Agents ran one after another, each handing off to the next (recorded before runners).';
  return {
    chain: 'Agents ran one after another; handoffs and messages optional.',
    concurrent: 'Agents ran at once; handoffs and messages optional.',
    external: 'An external system drove the agents through their endpoints.',
  }[rs.runner];
}

function BranchMenu({ runId, detail, siblings }: { runId: string; detail: BranchDetail; siblings: Branch[] }) {
  const nav = useNavigate();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild><Button size="sm"><GitBranch />Branches <span className="font-mono text-faint">{siblings.length || 1}</span><ChevronDown /></Button></DropdownMenuTrigger>
      <DropdownMenuContent className="w-80">
        <DropdownMenuLabel>Branches of this run</DropdownMenuLabel>
        {branchTree(siblings).map(({ branch: b, depth }) => (
          <DropdownMenuItem key={b.id} onSelect={() => nav(`/runs/${runId}/${b.id}`)} className={b.id === detail.id ? 'bg-accent-soft' : ''}>
            <span style={{ paddingLeft: depth * 12 }} className="font-mono text-2xs text-muted">{depth ? '↳' : '●'}</span>
            <StatusChip status={b.status} /><span className="font-mono text-xs">{b.id.slice(0, 8)}</span>
            <span className="min-w-0 flex-1 truncate text-xs text-muted">{b.fork_note ?? ''}</span><ApproximateMark of={b} />
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function CompareMenu({ detail, siblings }: { detail: BranchDetail; siblings: Branch[] }) {
  const nav = useNavigate();
  const others = siblings.filter((b) => b.id !== detail.id && (b.status === 'passed' || b.status === 'failed'));
  const why = detail.status === 'running' ? 'Wait for this run to finish.' : others.length === 0 ? 'No other finished branch of this run yet. Press ⌘K to compare with another run.' : '';
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild><Button size="sm" disabled={!!why} title={why}><GitCompare />Compare with…</Button></DropdownMenuTrigger>
      <DropdownMenuContent>
        {others.map((b) => (
          <DropdownMenuItem key={b.id} onSelect={() => nav(`/compare/${detail.id}/${b.id}`)}>
            <span className="font-mono text-xs">{b.id.slice(0, 8)}</span><StatusChip status={b.status} />
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function Stats({ detail }: { detail: BranchDetail }) {
  const sm = summarize(detail, detail.events);
  const st = runStats(detail.events, detail.start_seq);
  const cost = formatCost(sm.cost_usd, sm.cost_source, teamProviders(detail.team).some(costIsUnknown));
  return (
    <div className="grid grid-cols-3 gap-x-5 gap-y-2 sm:flex sm:flex-wrap sm:items-end">
      <Stat label={`Seconds${sm.live ? ' so far' : ''}`} value={sm.seconds != null ? sm.seconds.toFixed(1) : '–'} />
      <Stat label="Tokens" value={(detail.metrics?.tokens ?? sm.tokens).toLocaleString()} />
      <Stat label="Cost" value={cost} title={cost} />
      <Stat label="Steps" value={st.steps} />
      <Stat label="Reads · writes" value={`${st.reads} · ${st.writes}`} />
      <Stat label="Refused / errors" value={st.abnormal ? `⚠ ${st.abnormal}` : '0'} abnormal={st.abnormal > 0} />
      {st.check && <Stat className="col-span-2" label="Check" value={<StatusMark className="px-0 text-sm" mark={checkMark(st.check)} />} />}
      {st.safety && <Stat label="Safety" value={safetyLine(st.safety)} abnormal={st.safety.violated > 0} />}
      {sm.scope && <span className="col-span-3 text-xs text-muted sm:ml-auto">{sm.scope}</span>}
    </div>
  );
}

function Header({ runId, detail, siblings, step }: { runId: string; detail: BranchDetail; siblings: Branch[]; step: number | null }) {
  const task = taskOf(detail.events);
  const scenario = recordedLine((detail.events.find((e) => e.kind === 'run_start')?.payload as RunStartPayload | undefined)?.scenario);
  const world = worldLine(detail.events);
  const runner = runnerLine(detail.events);
  const driven = drivenLine(detail.events);
  return (
    <PageHeader mono title={task ? `task ${task}` : 'Run'}
      status={<><StatusChip status={detail.status} /><ApproximateMark of={detail} />{driven && <Tag>{driven}</Tag>}<span className="font-mono text-xs text-faint">{detail.id.slice(0, 8)}</span></>}
      actions={<>
        <Button size="sm" variant="primary" asChild><Link to={withStep(`/world/${runId}/${detail.id}`, step)}><Factory />World</Link></Button>
        <Button size="sm" asChild><Link to={withStep(`/control/${runId}/${detail.id}`, step)}><Workflow />Process view</Link></Button>
        <BranchMenu runId={runId} detail={detail} siblings={siblings} /><CompareMenu detail={detail} siblings={siblings} />
      </>}
      meta={<>{detail.fork_note && <span>{detail.fork_note}</span>}{world && <span className="truncate" title={world}>{world}</span>}{runner && <span>{runner}</span>}{scenario && <span className="text-warn">▼ {scenario}</span>}</>}>
      <Stats detail={detail} />
    </PageHeader>
  );
}

function Side({ runId, detail, event, forking, setForking, autoRows, choice }: {
  runId: string; detail: BranchDetail; event: Event | null; forking: boolean; setForking: (f: boolean) => void; autoRows: boolean; choice: ForkChoice;
}) {
  const nav = useNavigate();
  const canFork = !!event && event.kind !== 'check' && event.kind !== 'run_start';
  return (
    <Panel className="lg:sticky lg:top-[52px] lg:max-h-[calc(100vh-92px)] lg:overflow-auto">
      <PanelHeader title="Step">
        {event && <Button size="sm" variant={forking ? 'default' : 'primary'} onClick={() => setForking(!forking)} disabled={!canFork}
          title={canFork ? 'Change the team and rerun from this step' : 'Pick an agent step to rerun from'}>Change and rerun from here</Button>}
      </PanelHeader>
      {forking && event && (
        <div className="border-b border-line bg-raised">
          <ForkPanel key={`${detail.id}:${event.seq}`} branch={detail} seq={event.seq} choice={choice} onClose={() => setForking(false)}
            onForked={(r) => nav(`/runs/${runId}/${r.branch_id}`, { state: { notice: r.moved_from != null ? `Restarted from step ${r.start_seq} instead of ${r.moved_from}: ${r.reason ?? ''}` : null } })} />
        </div>
      )}
      <Inspector branchId={detail.id} event={event} team={detail.team} autoRows={autoRows} events={detail.events} />
    </Panel>
  );
}

function Main({ detail, selected, onSelect, models, setModels }: {
  detail: BranchDetail; selected: number | null; onSelect: (s: number) => void; models: boolean; setModels: (m: boolean) => void;
}) {
  const lanes = useMemo(() => buildLanes(detail.events, detail.team, detail.start_seq), [detail]);
  const live = detail.status === 'running';
  const now = Date.now();
  const span = spanOf(lanes);
  const pauses = useMemo(() => pausesOf(detail.events, now), [detail.events, now]);
  return (
    <div className="flex min-w-0 flex-col gap-3">
      <Panel>
        <Tabs defaultValue="timeline">
          <div className="flex items-center gap-3 border-b border-line pl-3">
            <h2 className="text-xs font-semibold text-muted">Actions{detail.status === 'running' ? ' · live' : ''}</h2>
            <TabsList className="border-b-0"><TabsTrigger value="timeline">Timeline</TabsTrigger><TabsTrigger value="log">Event log</TabsTrigger></TabsList>
          </div>
          <TabsContent value="timeline"><ActionTimeline lanes={lanes} team={detail.team} span={span} selected={selected} onSelect={onSelect} models={models} onModels={setModels} live={live} pauses={pauses} /></TabsContent>
          <TabsContent value="log"><EventLog lanes={lanes} span={span} selected={selected} onSelect={onSelect} models={models} /></TabsContent>
        </Tabs>
      </Panel>
      <Panel><PanelHeader title="Trace" meta="the selected step's turn" /><Waterfall events={detail.events} selected={selected} onSelect={onSelect} /></Panel>
    </div>
  );
}

/** Selection (the address's `?step=`, shared with the process and world views), fork panel and notice, reset whenever the branch changes. */
function useRunView(branchId: string, order: number[]) {
  const [sp] = useSearchParams();
  const loc = useLocation();
  const [selected, setSelected] = useStepParam();
  const [forking, setForking] = useState(sp.get('fork') === '1');
  const [notice, setNotice] = useState<string | null>((loc.state as { notice?: string } | null)?.notice ?? null);
  useEffect(() => {
    setForking(sp.get('fork') === '1');
    setNotice((loc.state as { notice?: string } | null)?.notice ?? null);
  }, [branchId]); // eslint-disable-line react-hooks/exhaustive-deps
  useStepKeys(order, setSelected);
  return { selected, setSelected, forking, setForking, notice, setNotice, autoRows: sp.get('rows') === '1', choice: forkChoiceOf(sp.get('change')) };
}

export default function Run() {
  const { runId = '', branchId = '' } = useParams();
  const { detail, siblings, error, reload } = useBranch(branchId, runId);
  const [models, setModels] = useState(true);
  const order = useMemo(() => (detail ? stepOrder(buildLanes(detail.events, detail.team, detail.start_seq), models) : []), [detail, models]);
  const v = useRunView(branchId, order);
  if (!detail) {
    return <><PageHeader title="Run" /><PageBody>{error ? <ErrorState error={error} onRetry={reload} /> : <Panel><LoadingState label="Loading the run's steps…" rows={5} /></Panel>}</PageBody></>;
  }
  const event = detail.events.find((e) => e.seq === v.selected) ?? null;
  return (
    <>
      <Header runId={runId} detail={detail} siblings={siblings} step={v.selected} />
      <PageBody>
        {v.notice && <Notice onDismiss={() => v.setNotice(null)}>{v.notice}</Notice>}
        {error && <ErrorState error={error} onRetry={reload} />}
        {detail.error && <ErrorState error={detail.error} hint="The run stopped with this error. Its steps up to here are recorded." />}
        <Split storageKey="arena.run.side" main={<Main detail={detail} selected={v.selected} onSelect={v.setSelected} models={models} setModels={setModels} />}
          side={<Side runId={runId} detail={detail} event={event} forking={v.forking} setForking={v.setForking} autoRows={v.autoRows} choice={v.choice} />} />
        {detail.metrics && <Panel><PanelHeader title="Deployment estimate" meta="time in production, arena overhead, tokens and cost per agent" /><MetricsPanel m={detail.metrics} /></Panel>}
      </PageBody>
    </>
  );
}
