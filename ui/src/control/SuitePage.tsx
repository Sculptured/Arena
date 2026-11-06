import { useEffect, useState } from 'react';
import { Link, useParams } from 'react-router-dom';
import { suite as fetchSuite } from './api';
import { Figure, StatusMark } from './Marks';
import { type GridRow, checkRows, fmt, grid, pct, spreadLine, suiteMark } from './suite';
import type { SuiteReport, TaskSpread } from './types';
import './control.css';

/** While a suite runs, its report is read again this often. */
const POLL_MS = 2000;

function Overview({ r }: { r: SuiteReport }) {
  const s = r.summary;
  return (
    <div className="hmi-figures panel">
      <Figure label="Pass rate" value={`${pct(s.pass_rate)} (${s.passed}/${s.scored})`} title="Scored runs that passed every check, safety included" />
      <Figure label="Score" value={pct(s.score)} title="Goal checks passed (safety checks counted apart)" />
      <Figure label="Safety violations" value={`${s.safety_violations} in ${s.runs_with_violation} runs`} abnormal={s.safety_violations > 0} />
      <Figure label="Unscored" value={`${s.unscored}`} abnormal={s.unscored > 0} title="Runs that ended without a check (infrastructure error, interrupted)" />
      <Figure label="Time per run" value={spreadLine(s.measured_s, 's')} />
      <Figure label="Estimated in production" value={spreadLine(s.estimated_s, 's')} />
      <Figure label="Tokens per run" value={spreadLine(s.tokens, 'tok')} />
      <Figure label="Cost per run" value={spreadLine(s.cost_usd, 'usd')} />
      <Figure label="Peak context" value={spreadLine(s.peak_context, 'tok')} />
      <Figure label="Time to finish" value={fmt(r.finished_s, 's')} />
    </div>
  );
}

function GridLine({ row }: { row: GridRow }) {
  return (
    <tr>
      <th className="mono">{row.task}</th>
      {row.cells.map((c) => (
        <td key={c.repeat} title={c.title}>
          {c.runId && c.branchId
            ? <Link className="hmi-cell" to={`/control/${c.runId}/${c.branchId}`}>
              <StatusMark mark={c.mark} />{c.checks && <span className="mono muted"> {c.checks}</span>}
              {c.unsafe && <span className="hmi-mark abnormal"> ◆ unsafe</span>}
            </Link>
            : <StatusMark mark={c.mark} />}
        </td>
      ))}
      <td className="mono">{row.passed}/{row.scored}</td>
      <td className={`mono${row.violations ? ' hmi-abnormal-text' : ''}`}>{row.violations}</td>
    </tr>
  );
}

function SpreadTable({ spread }: { spread: TaskSpread[] }) {
  return (
    <table className="hmi-table panel">
      <thead><tr><th>Scenario</th><th>Runs</th><th>Checks passed</th><th>Time</th><th>Estimated</th><th>Tokens</th><th>Cost</th><th>Peak context</th></tr></thead>
      <tbody>{spread.map((t) => (
        <tr key={t.task_id}>
          <th className="mono">{t.task_id}</th><td className="mono">{t.runs}</td>
          <td className="mono">{spreadLine(t.checks_passed, 'n')}</td><td className="mono">{spreadLine(t.measured_s, 's')}</td>
          <td className="mono">{spreadLine(t.estimated_s, 's')}</td><td className="mono">{spreadLine(t.tokens, 'tok')}</td>
          <td className="mono">{spreadLine(t.cost_usd, 'usd')}</td><td className="mono">{spreadLine(t.peak_context, 'tok')}</td>
        </tr>
      ))}</tbody>
    </table>
  );
}

function Checks({ r }: { r: SuiteReport }) {
  return (
    <table className="hmi-table panel">
      <thead><tr><th>Scenario</th><th>Check</th><th>Kind</th><th>Passed</th><th>Rate</th></tr></thead>
      <tbody>{r.tasks.flatMap((t) => checkRows(r.checks, t).map((c) => {
        const bad = c.safety && c.passed < c.runs;
        return (
          <tr key={`${t}/${c.name}`}>
            <th className="mono">{t}</th><td className="mono">{c.name}</td>
            <td>{c.safety ? <span className={`hmi-mark${bad ? ' abnormal' : ''}`}>◆ safety</span> : 'goal'}</td>
            <td className="mono">{c.passed}/{c.runs}</td><td className="mono">{pct(c.pass_rate)}</td>
          </tr>
        );
      }))}</tbody>
    </table>
  );
}

export default function SuitePage() {
  const { id = '' } = useParams();
  const [r, setR] = useState<SuiteReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const load = () => fetchSuite(id).then((v) => {
      if (!alive) return;
      setR(v); setError(null);
      if (v.status === 'running') timer = setTimeout(load, POLL_MS);
    }).catch((e) => alive && setError(String(e.message ?? e)));
    load();
    return () => { alive = false; clearTimeout(timer); };
  }, [id]);

  if (!r) return <><header className="page-head"><h1>Suite</h1></header><main className="content">{error ? <div className="notice error">{error}</div> : <p className="muted">Loading…</p>}</main></>;
  const rows = grid(r);
  return (
    <>
      <header className="page-head">
        <h1>{r.name}</h1><StatusMark mark={suiteMark(r.status)} />
        <span className="muted small">{r.team.name} · {r.benchmark} · {r.tasks.length} scenarios × {r.repeats} repeats · {r.runner}</span>
        <span className="spacer" /><Link className="btn btn-sm" to="/suites">All suites</Link>
      </header>
      <main className="content wide hmi">
        {error && <div className="notice error">{error}</div>}
        {r.error && <div className="notice error">⚠ {r.error}</div>}
        <Overview r={r} />
        <h2 className="hmi-h2">Scenarios × repeats</h2>
        <table className="hmi-table panel hmi-grid">
          <thead><tr><th>Scenario</th>{Array.from({ length: r.repeats }, (_, i) => <th key={i}>Repeat {i + 1}</th>)}<th>Passed</th><th>Safety</th></tr></thead>
          <tbody>{rows.map((row) => <GridLine key={row.task} row={row} />)}</tbody>
        </table>
        <h2 className="hmi-h2">Checks</h2>
        <Checks r={r} />
        <h2 className="hmi-h2">Spread across repeats</h2>
        <SpreadTable spread={r.spread} />
      </main>
    </>
  );
}
