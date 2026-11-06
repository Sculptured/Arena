import { useEffect, useState } from 'react';
import { Link, useParams } from 'react-router-dom';
import { compareSuites } from './api';
import { StatusMark } from './Marks';
import { type Unit, change, fmt, pct, pts, verdictMark } from './suite';
import type { Delta, SuiteComparison } from './types';
import './control.css';

function FigureRow({ label, d, unit }: { label: string; d: Delta; unit: Unit | 'rate' }) {
  const show = (x: number | null) => (unit === 'rate' ? pct(x) : fmt(x, unit));
  return (
    <tr>
      <th>{label}</th><td className="mono">{show(d.a)}</td><td className="mono">{show(d.b)}</td>
      <td className="mono">{unit === 'rate' ? pts(d.change) : change(d.change, unit)}</td>
      <td><StatusMark mark={verdictMark(d.verdict)} /></td>
    </tr>
  );
}

export default function SuiteComparePage() {
  const { a = '', b = '' } = useParams();
  const [c, setC] = useState<SuiteComparison | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => { compareSuites(a, b).then(setC).catch((e) => setError(String(e.message ?? e))); }, [a, b]);
  if (!c) return <><header className="page-head"><h1>Compare suites</h1></header><main className="content">{error ? <div className="notice error">{error}</div> : <p className="muted">Loading…</p>}</main></>;
  return (
    <>
      <header className="page-head">
        <h1>Compare suites</h1><StatusMark mark={verdictMark(c.verdict)} />
        <span className="muted small">B against A, from checks and safety; time and cost flagged beyond 10%</span>
      </header>
      <main className="content wide hmi">
        <table className="hmi-table panel">
          <thead><tr><th /><th>A: <Link to={`/suites/${c.a.id}`}>{c.a.name}</Link></th><th>B: <Link to={`/suites/${c.b.id}`}>{c.b.name}</Link></th><th>Change</th><th>Verdict</th></tr></thead>
          <tbody>
            <tr><th>Team</th><td>{c.a.team}</td><td>{c.b.team}</td><td /><td /></tr>
            <tr><th>Scored runs</th><td className="mono">{c.a.scored}</td><td className="mono">{c.b.scored}</td><td /><td /></tr>
            <FigureRow label="Pass rate" d={c.pass_rate} unit="rate" />
            <FigureRow label="Score" d={c.score} unit="rate" />
            <FigureRow label="Safety violations per run" d={c.safety} unit="n" />
            <FigureRow label="Time per run" d={c.measured_s} unit="s" />
            <FigureRow label="Estimated in production" d={c.estimated_s} unit="s" />
            <FigureRow label="Tokens per run" d={c.tokens} unit="tok" />
            <FigureRow label="Cost per run" d={c.cost_usd} unit="usd" />
            <FigureRow label="Peak context" d={c.peak_context} unit="tok" />
          </tbody>
        </table>
        <h2 className="hmi-h2">Checks</h2>
        <table className="hmi-table panel">
          <thead><tr><th>Scenario</th><th>Check</th><th>A</th><th>B</th><th>Change</th><th>Verdict</th></tr></thead>
          <tbody>{c.checks.map((d) => (
            <tr key={`${d.task_id}/${d.name}`}>
              <th className="mono">{d.task_id}</th>
              <td className="mono">{d.name}{d.safety && <span className="muted"> ◆ safety</span>}</td>
              <td className="mono">{pct(d.a)}</td><td className="mono">{pct(d.b)}</td><td className="mono">{pts(d.change)}</td>
              <td><StatusMark mark={verdictMark(d.verdict)} /></td>
            </tr>
          ))}</tbody>
        </table>
      </main>
    </>
  );
}
