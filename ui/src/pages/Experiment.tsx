import { useEffect, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';

type Trial = {
  run: string; branch: string; checks: { passed: number; total: number; failed?: string[] };
  measured_s: number; estimated_s: number | null; overhead_s: number;
  cost: { usd: number | null; label: string; source?: string };
  milestones: { name: string; t_s: number | null }[];
  context: Record<string, { t_s: number; prompt_tokens: number }[]>;
  messages: { from: string; to: string; count: number }[];
  topics?: Record<string, number>;
};
type Report = { sample: boolean; context_limit?: number; source?: string; designs: Record<string, { label: string; runs: Trial[] }> };
const tabs = ['Outcomes', 'Timing', 'Context', 'Coordination', 'Deployment'] as const;
type Tab = typeof tabs[number];
const mean = (values: number[]) => values.reduce((a, b) => a + b, 0) / values.length;
const num = (value: number) => value.toLocaleString(undefined, { maximumFractionDigits: 1 });

function parseReport(value: unknown): Report {
  const report = value as Report;
  if (!report || typeof report.designs !== 'object' || !report.designs || !Object.keys(report.designs).length)
    throw new Error('Choose an Arena showcase metrics export with recorded designs.');
  for (const d of Object.values(report.designs)) {
    if (!d.runs?.length || d.runs.some(r => !r.branch || !r.run || !r.checks || !r.context || !r.milestones))
      throw new Error('The export is missing recorded runs, checks, context, or milestones.');
  }
  return report;
}

function Context({ run, limit }: { run: Trial; limit?: number }) {
  const series = Object.entries(run.context).filter(([, points]) => points.length);
  const all = series.flatMap(([, points]) => points);
  if (!all.length) return <p className="muted">No per-call context measurements recorded.</p>;
  const xmax = Math.max(1, ...all.map(p => p.t_s));
  const ymax = Math.max(1, limit ?? 0, ...all.map(p => p.prompt_tokens));
  return <>
    <svg viewBox="0 0 600 180" role="img" aria-label="Input tokens per model call over elapsed seconds" className="context-plot">
      {[0, 0.5, 1].map(f => <g key={f}><line x1="60" x2="585" y1={145 - f * 125} y2={145 - f * 125} stroke="var(--line)" /><text x="52" y={149 - f * 125} textAnchor="end">{num(ymax * f / 1000)}k</text></g>)}
      {series.map(([agent, points], index) => <polyline key={agent} fill="none" stroke={`var(--series-${index % 5})`} strokeWidth="2.5"
        points={points.map(p => `${60 + 525 * p.t_s / xmax},${145 - 125 * p.prompt_tokens / ymax}`).join(' ')} />)}
      <text x="60" y="173">0 s</text><text x="585" y="173" textAnchor="end">{num(xmax)} s elapsed</text>
    </svg>
    <div className="context-key">{series.map(([agent, points], index) => <span key={agent} style={{ borderColor: `var(--series-${index % 5})` }}>{agent} · peak {num(Math.max(...points.map(p => p.prompt_tokens)))} tokens</span>)}</div>
  </>;
}

function Evidence({ tab, trial, limit }: { tab: Tab; trial: Trial; limit?: number }) {
  if (tab === 'Context') return <Context run={trial} limit={limit} />;
  if (tab === 'Timing') return <div className="milestone-list">{trial.milestones.map(m => <div key={m.name}><span>{m.name}</span><strong className="mono">{m.t_s == null ? 'Not reached' : `${num(m.t_s)} s`}</strong></div>)}</div>;
  if (tab === 'Coordination') return <>
    <div className="milestone-list">{trial.messages.map((m, i) => <div key={i}><span>{m.from} → {m.to}</span><strong>{m.count} messages</strong></div>)}</div>
    {Object.keys(trial.topics ?? {}).length > 0 && <><h3>Published events</h3><div className="milestone-list">{Object.entries(trial.topics!).map(([topic, n]) => <div key={topic}><span>{topic}</span><strong>{n}</strong></div>)}</div></>}
    {!trial.messages.length && !Object.keys(trial.topics ?? {}).length && <p className="muted">No inter-agent messages or topic events recorded.</p>}
  </>;
  if (tab === 'Deployment') return <dl className="deployment-facts">
    <dt>Measured wall time</dt><dd>{num(trial.measured_s)} s</dd>
    <dt>Arena overhead</dt><dd>{num(trial.overhead_s)} s</dd>
    <dt>Time excluding overhead</dt><dd>{trial.estimated_s == null ? 'Unknown' : `${num(trial.estimated_s)} s`}</dd>
    <dt>Cost</dt><dd>{trial.cost.usd == null ? 'Unknown' : `$${trial.cost.usd.toFixed(2)}`}</dd>
    <dt>Cost provenance</dt><dd>{trial.cost.source?.replaceAll('_', ' ') ?? trial.cost.label}</dd>
  </dl>;
  return <><h3>Unmet checks</h3>{trial.checks.failed?.map(check => <p className="failed-check" key={check}><span aria-hidden="true">×</span>{check.replaceAll('_', ' ')}</p>)}</>;
}

export default function Experiment() {
  const [sp, setSp] = useSearchParams();
  const tab = tabs.includes(sp.get('view') as Tab) ? sp.get('view') as Tab : 'Outcomes';
  const [report, setReport] = useState<Report | null>(null);
  const [error, setError] = useState('');
  const source = sp.get('report');
  useEffect(() => {
    if (!source) return;
    let cancelled = false;
    setReport(null); setError('');
    if (!source.startsWith('/') || source.startsWith('//')) { setError('Report URL must be a path on this Arena server.'); return; }
    fetch(source).then(r => { if (!r.ok) throw new Error(`Report request failed: ${r.status}`); return r.json(); })
      .then(parseReport).then(r => { if (!cancelled) setReport(r); }).catch(e => { if (!cancelled) setError(String(e.message)); });
    return () => { cancelled = true; };
  }, [source]);
  return <>
    <header className="page-head"><h1>Experiment report</h1><span className="spacer" /><span className="chip">Recorded evidence</span></header>
    <main className="content experiment">
      <div className="experiment-intro"><div><div className="label">Test · inspect · compare</div><h2>One failure. Three agent architectures.</h2><p className="muted">Experimental factory setup · fictional business records · recorded agent outcomes</p></div>
        <label className="btn">Open metrics export<input type="file" accept=".json" className="report-upload" onChange={async e => {
          const file = e.target.files?.[0]; if (!file) return;
          try { const value = parseReport(JSON.parse(await file.text())); setReport(value); setError(''); }
          catch (err) { setError(String(err)); }
        }} /></label></div>
      {error && <div className="notice error" role="alert">{error}</div>}
      {!report && !error && <p className="muted">{source ? 'Loading report…' : 'Open metrics.json exported by the factory showcase to compare recorded runs.'}</p>}
      {report && <>
        <div className="notice">{report.sample ? 'SAMPLE DATA — NOT RESULTS. ' : ''}Small experimental sample. Overview values are means across the listed runs; detailed evidence below uses each design’s first run. These are not production guarantees.</div>
        <div className="experiment-tabs" role="tablist" aria-label="Report view">{tabs.map(t => <button key={t} role="tab" aria-selected={tab === t} className={t === tab ? 'active' : ''}
          onClick={() => setSp(prev => { const next = new URLSearchParams(prev); next.set('view', t); return next; })}>{t}</button>)}</div>
        <div className="experiment-grid">{Object.entries(report.designs).map(([key, design]) => {
          const first = design.runs[0];
          const passed = mean(design.runs.map(r => r.checks.passed));
          const times = design.runs.map(r => r.estimated_s).filter((n): n is number => n != null);
          const costs = design.runs.map(r => r.cost.usd).filter((n): n is number => n != null);
          return <article className="panel experiment-design" key={key}>
            <div className="design-top"><span className="design-letter">{key}</span><div><h2>{design.label.replace(/^[A-Z],\s*/, '')}</h2><span className="muted small">{design.runs.length} recorded run{design.runs.length !== 1 ? 's' : ''}</span></div></div>
            <div className="experiment-score"><strong>{num(passed)}<small> / {first.checks.total}</small></strong><span>mean checks passed</span></div>
            <div className="score-track"><span style={{ width: `${100 * passed / first.checks.total}%` }} /></div>
            <div className="experiment-stats"><span><b>{times.length === design.runs.length ? `${num(mean(times) / 60)} min` : 'Unknown'}</b>excluding overhead</span><span><b>{costs.length === design.runs.length ? `$${mean(costs).toFixed(2)}` : 'Unknown'}</b>API-equivalent estimate</span></div>
            <div className="evidence-title"><strong>{tab}</strong><span className="muted small">First recorded run</span></div>
            <Evidence tab={tab} trial={first} limit={report.context_limit} />
            <Link className="btn evidence-link" to={`/runs/${first.run}/${first.branch}`}>Inspect recorded run →</Link>
          </article>;
        })}</div>
        <p className="muted small">Source: Arena metrics export. Timing milestones include experiment overhead; deployment estimates subtract measured Arena overhead. Context plots show input tokens per call, not cumulative usage.</p>
      </>}
    </main>
  </>;
}
