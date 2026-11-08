import { useEffect, useState } from 'react';
import { Link, useParams } from 'react-router-dom';
import { PageBody, PageHeader } from '../components/ui/page';
import { Panel, PanelHeader } from '../components/ui/panel';
import { ErrorState, LoadingState } from '../components/ui/state';
import { StatusMark, Tag } from '../components/ui/status';
import { RowTh, Table, Td, Th, Tr } from '../components/ui/table';
import { compareSuites } from './api';
import { SuiteWhatMatters } from '../review/WhatMatters';
import { type Unit, change, fmt, pct, pts, suiteMark, verdictMark } from './suite';
import type { Delta, SuiteComparison } from './types';

function FigureRow({ label, d, unit }: { label: string; d: Delta; unit: Unit | 'rate' }) {
  const show = (x: number | null) => (unit === 'rate' ? pct(x) : fmt(x, unit));
  return (
    <Tr>
      <RowTh>{label}</RowTh><Td className="text-right font-mono text-xs">{show(d.a)}</Td><Td className="text-right font-mono text-xs">{show(d.b)}</Td>
      <Td className="text-right font-mono text-xs">{unit === 'rate' ? pts(d.change) : change(d.change, unit)}</Td>
      <Td><StatusMark mark={verdictMark(d.verdict)} /></Td>
    </Tr>
  );
}

function Figures({ c }: { c: SuiteComparison }) {
  return (
    <Panel>
      <PanelHeader title="Figures" meta="per run; B against A" />
      <Table>
        <thead><tr><Th /><Th className="text-right">A</Th><Th className="text-right">B</Th><Th className="text-right">Change</Th><Th>Verdict</Th></tr></thead>
        <tbody>
          <Tr><RowTh>Team</RowTh><Td className="text-right">{c.a.team}</Td><Td className="text-right">{c.b.team}</Td><Td /><Td /></Tr>
          <Tr><RowTh>Scored runs</RowTh><Td className="text-right font-mono text-xs">{c.a.scored}</Td><Td className="text-right font-mono text-xs">{c.b.scored}</Td><Td /><Td /></Tr>
          <FigureRow label="Pass rate" d={c.pass_rate} unit="rate" />
          <FigureRow label="Score" d={c.score} unit="rate" />
          <FigureRow label="Safety violations per run" d={c.safety} unit="n" />
          <FigureRow label="Time per run" d={c.measured_s} unit="s" />
          <FigureRow label="Estimated in production" d={c.estimated_s} unit="s" />
          <FigureRow label="Tokens per run" d={c.tokens} unit="tok" />
          <FigureRow label="Cost per run" d={c.cost_usd} unit="usd" />
          <FigureRow label="Peak context" d={c.peak_context} unit="tok" />
        </tbody>
      </Table>
    </Panel>
  );
}

function Checks({ c }: { c: SuiteComparison }) {
  return (
    <Panel>
      <PanelHeader title="Checks" meta="pass rate of each check in A and B" />
      <Table>
        <thead><tr><Th>Scenario</Th><Th>Check</Th><Th className="text-right">A</Th><Th className="text-right">B</Th><Th className="text-right">Change</Th><Th>Verdict</Th></tr></thead>
        <tbody>{c.checks.map((d) => (
          <Tr key={`${d.task_id}/${d.name}`}>
            <RowTh className="font-mono text-xs">{d.task_id}</RowTh>
            <Td><span className="flex items-center gap-1.5 font-mono text-xs">{d.name}{d.safety && <Tag>◆ safety</Tag>}</span></Td>
            <Td className="text-right font-mono text-xs">{pct(d.a)}</Td><Td className="text-right font-mono text-xs">{pct(d.b)}</Td><Td className="text-right font-mono text-xs">{pts(d.change)}</Td>
            <Td><StatusMark mark={verdictMark(d.verdict)} /></Td>
          </Tr>
        ))}</tbody>
      </Table>
    </Panel>
  );
}

function Sides({ c }: { c: SuiteComparison }) {
  const side = (label: string, s: SuiteComparison['a']) => (
    <Panel className="flex flex-wrap items-center gap-x-3 gap-y-1 px-3 py-2">
      <span className="font-mono text-xs text-muted">{label}</span>
      <Link className="font-medium hover:underline" to={`/suites/${s.id}`}>{s.name}</Link>
      <StatusMark mark={suiteMark(s.status)} /><span className="text-xs text-muted">{s.team} · {s.scored} scored runs</span>
    </Panel>
  );
  return <div className="grid gap-3 lg:grid-cols-2">{side('A', c.a)}{side('B', c.b)}</div>;
}

export default function SuiteComparePage() {
  const { a = '', b = '' } = useParams();
  const [c, setC] = useState<SuiteComparison | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => { setC(null); compareSuites(a, b).then(setC).catch((e) => setError(String(e.message ?? e))); }, [a, b]);
  if (!c) return <><PageHeader title="Compare suites" /><PageBody>{error ? <ErrorState error={error} /> : <Panel><LoadingState label="Comparing the two suites…" rows={6} /></Panel>}</PageBody></>;
  return (
    <>
      <PageHeader title="Compare suites" status={<StatusMark mark={verdictMark(c.verdict)} />}
        meta="B against A, judged from checks and safety; time and cost flagged beyond 10%." />
      <PageBody>
        <Sides c={c} />
        <SuiteWhatMatters a={a} b={b} />
        <Figures c={c} />
        <Checks c={c} />
      </PageBody>
    </>
  );
}
