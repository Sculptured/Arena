import { GitCompare } from 'lucide-react';
import { useEffect, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { Button } from '../components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuTrigger } from '../components/ui/dropdown-menu';
import { PageBody, PageHeader, Stat } from '../components/ui/page';
import { Panel, PanelHeader } from '../components/ui/panel';
import { ErrorState, LoadingState } from '../components/ui/state';
import { StatusMark, Tag } from '../components/ui/status';
import { RowTh, Table, Td, Th, Tr } from '../components/ui/table';
import { cn } from '@/lib/cn';
import { RUNNERS } from '../suites/NewSuiteDialog';
import { suite as fetchSuite, suites as fetchSuites } from './api';
import { type GridRow, checkRows, fmt, grid, pct, spreadLine, suiteMark } from './suite';
import type { ListedSuite, SuiteReport, TaskSpread } from './types';

/** While a suite runs, its report is read again this often. */
const POLL_MS = 2000;

function Overview({ r }: { r: SuiteReport }) {
  const s = r.summary;
  return (
    <Panel className="grid grid-cols-2 gap-x-6 gap-y-3 p-3 sm:grid-cols-3 lg:grid-cols-5">
      <Stat label="Pass rate" value={`${pct(s.pass_rate)} (${s.passed}/${s.scored})`} title="Scored runs that passed every check, safety included" />
      <Stat label="Score" value={pct(s.score)} title="Goal checks passed (safety checks counted apart)" />
      <Stat label="Safety violations" value={`${s.safety_violations ? '◆ ' : ''}${s.safety_violations} in ${s.runs_with_violation} runs`} abnormal={s.safety_violations > 0} />
      <Stat label="Unscored" value={`${s.unscored ? '⚠ ' : ''}${s.unscored}`} abnormal={s.unscored > 0} title="Runs that ended without a check (infrastructure error, interrupted)" />
      <Stat label="Time to finish" value={fmt(r.finished_s, 's')} />
      <Stat label="Time per run" value={spreadLine(s.measured_s, 's')} />
      <Stat label="Estimated in production" value={spreadLine(s.estimated_s, 's')} />
      <Stat label="Tokens per run" value={spreadLine(s.tokens, 'tok')} />
      <Stat label="Cost per run" value={spreadLine(s.cost_usd, 'usd')} />
      <Stat label="Peak context" value={spreadLine(s.peak_context, 'tok')} />
    </Panel>
  );
}

function GridLine({ row }: { row: GridRow }) {
  return (
    <Tr>
      <RowTh className="font-mono text-xs">{row.task}</RowTh>
      {row.cells.map((c) => (
        <Td key={c.repeat} title={c.title}>
          {c.runId && c.branchId
            ? <Link className="inline-flex items-center gap-1.5 rounded-sm hover:underline" to={`/control/${c.runId}/${c.branchId}`}>
              <StatusMark mark={c.mark} />{c.checks && <span className="font-mono text-2xs text-muted">{c.checks}</span>}
              {c.unsafe && <Tag tone="alarm">◆ unsafe</Tag>}
            </Link>
            : <StatusMark mark={c.mark} />}
        </Td>
      ))}
      <Td className="text-right font-mono text-xs">{row.passed}/{row.scored}</Td>
      <Td className={cn('text-right font-mono text-xs', row.violations && 'font-semibold text-alarm')}>{row.violations ? `◆ ${row.violations}` : 0}</Td>
    </Tr>
  );
}

function Grid({ r }: { r: SuiteReport }) {
  return (
    <Panel>
      <PanelHeader title="Scenarios × repeats" meta="each cell opens the run's process view" />
      <Table>
        <thead><tr><Th>Scenario</Th>{Array.from({ length: r.repeats }, (_, i) => <Th key={i}>Repeat {i + 1}</Th>)}<Th className="text-right">Passed</Th><Th className="text-right">Safety</Th></tr></thead>
        <tbody>{grid(r).map((row) => <GridLine key={row.task} row={row} />)}</tbody>
      </Table>
    </Panel>
  );
}

function SpreadTable({ spread }: { spread: TaskSpread[] }) {
  const cell = (v: string) => <Td className="font-mono text-xs">{v}</Td>;
  return (
    <Panel>
      <PanelHeader title="Spread across repeats" meta="mean (min–max)" />
      <Table>
        <thead><tr><Th>Scenario</Th><Th>Runs</Th><Th>Checks passed</Th><Th>Time</Th><Th>Estimated</Th><Th>Tokens</Th><Th>Cost</Th><Th>Peak context</Th></tr></thead>
        <tbody>{spread.map((t) => (
          <Tr key={t.task_id}>
            <RowTh className="font-mono text-xs">{t.task_id}</RowTh>{cell(String(t.runs))}
            {cell(spreadLine(t.checks_passed, 'n'))}{cell(spreadLine(t.measured_s, 's'))}{cell(spreadLine(t.estimated_s, 's'))}
            {cell(spreadLine(t.tokens, 'tok'))}{cell(spreadLine(t.cost_usd, 'usd'))}{cell(spreadLine(t.peak_context, 'tok'))}
          </Tr>
        ))}</tbody>
      </Table>
    </Panel>
  );
}

function Checks({ r }: { r: SuiteReport }) {
  return (
    <Panel>
      <PanelHeader title="Checks" meta="goal checks, then safety checks" />
      <Table>
        <thead><tr><Th>Scenario</Th><Th>Check</Th><Th>Kind</Th><Th className="text-right">Passed</Th><Th className="text-right">Rate</Th></tr></thead>
        <tbody>{r.tasks.flatMap((t) => checkRows(r.checks, t).map((c) => {
          const bad = c.safety && c.passed < c.runs;
          return (
            <Tr key={`${t}/${c.name}`}>
              <RowTh className="font-mono text-xs">{t}</RowTh><Td className="font-mono text-xs">{c.name}</Td>
              <Td>{c.safety ? <Tag tone={bad ? 'alarm' : undefined}>◆ safety</Tag> : <span className="text-xs text-muted">goal</span>}</Td>
              <Td className="text-right font-mono text-xs">{c.passed}/{c.runs}</Td><Td className={cn('text-right font-mono text-xs', bad && 'font-semibold text-alarm')}>{pct(c.pass_rate)}</Td>
            </Tr>
          );
        }))}</tbody>
      </Table>
    </Panel>
  );
}

function useSuiteReport(id: string) {
  const [r, setR] = useState<SuiteReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    setR(null);
    const load = () => fetchSuite(id).then((v) => {
      if (!alive) return;
      setR(v); setError(null);
      if (v.status === 'running') timer = setTimeout(load, POLL_MS);
    }).catch((e) => alive && setError(String(e.message ?? e)));
    load();
    return () => { alive = false; clearTimeout(timer); };
  }, [id]);
  return { r, error };
}

function CompareWith({ id }: { id: string }) {
  const nav = useNavigate();
  const [others, setOthers] = useState<ListedSuite[] | null>(null);
  return (
    <DropdownMenu onOpenChange={(o) => { if (o && !others) fetchSuites().then((l) => setOthers(l.filter((s) => s.id !== id))).catch(() => setOthers([])); }}>
      <DropdownMenuTrigger asChild><Button size="sm"><GitCompare />Compare with…</Button></DropdownMenuTrigger>
      <DropdownMenuContent className="w-72">
        <DropdownMenuLabel>{others ? (others.length ? 'This suite (A) against' : 'No other suite yet') : 'Loading suites…'}</DropdownMenuLabel>
        {others?.map((s) => (
          <DropdownMenuItem key={s.id} onSelect={() => nav(`/suites/compare/${id}/${s.id}`)}>
            <span className="min-w-0 flex-1 truncate">{s.name}</span><StatusMark mark={suiteMark(s.status)} />
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export default function SuitePage() {
  const { id = '' } = useParams();
  const { r, error } = useSuiteReport(id);
  if (!r) return <><PageHeader title="Suite" /><PageBody>{error ? <ErrorState error={error} /> : <Panel><LoadingState label="Loading the suite report…" rows={6} /></Panel>}</PageBody></>;
  return (
    <>
      <PageHeader title={r.name} status={<StatusMark mark={suiteMark(r.status)} />}
        meta={<><span>{r.team.name}</span><span>{r.benchmark}</span><span>{r.tasks.length} scenarios × {r.repeats} repeats</span><span>runs made by: {RUNNERS[r.runner]}</span>{r.status === 'running' && <span>refreshing every 2 s</span>}</>}
        actions={<><Button size="sm" asChild><Link to="/suites">All suites</Link></Button><CompareWith id={r.id} /></>} />
      <PageBody>
        {error && <ErrorState error={error} hint="Showing the last report read." />}
        {r.error && <ErrorState error={r.error} hint="The suite stopped. The runs that ended are scored below." />}
        <Overview r={r} />
        <Grid r={r} />
        <Checks r={r} />
        <SpreadTable spread={r.spread} />
      </PageBody>
    </>
  );
}
