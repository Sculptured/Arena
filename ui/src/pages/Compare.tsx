import { useEffect, useState } from 'react';
import { useParams } from 'react-router-dom';
import { branch as fetchBranch, compare } from '../api';
import { StatusChip } from '../components';
import { formatCost, jsonDiff } from '../run/timeline';
import { pretty } from '../run/Inspector';
import type { CompareResponse, CompareSide, Event } from '../types';

function StepView({ id, ev }: { id: string; ev: Event | undefined }) {
  const p = (ev?.payload ?? {}) as Record<string, unknown>;
  return (
    <div className="panel cmp-col">
      <div className="cmp-top"><span className="mono">{id.slice(0, 8)}</span></div>
      {!ev ? <span className="muted">(no step — this branch ended)</span> : (
        <>
          <div className="small"><span className="mono">step {ev.seq}</span> · {ev.agent ?? 'run'} · <span className="chip">{ev.kind.replace('_', ' ')}</span>{typeof p.name === 'string' && <span className="mono"> {p.name}</span>}</div>
          {ev.kind === 'tool_call' ? <pre className="json">{pretty(p.args)}</pre> : <pre className="json">{pretty(p)}</pre>}
        </>
      )}
    </div>
  );
}

function Side({ id, s }: { id: string; s: CompareSide }) {
  return (
    <div className="panel cmp-col">
      <div className="cmp-top"><span className="mono">{id.slice(0, 8)}</span><StatusChip status={s.status} /></div>
      <dl className="kv">
        <dt>Seconds</dt><dd>{s.seconds != null ? s.seconds.toFixed(1) : '–'}</dd>
        <dt>Tokens</dt><dd>{s.tokens ?? '–'}</dd>
        <dt>Cost</dt><dd>{formatCost(s.cost_usd, s.cost_source)}</dd>
      </dl>
    </div>
  );
}

export default function Compare() {
  const { a = '', b = '' } = useParams();
  const [data, setData] = useState<CompareResponse | null>(null);
  const [evs, setEvs] = useState<[Event[], Event[]] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    setData(null); setEvs(null); setError(null);
    Promise.all([compare(a, b), fetchBranch(a), fetchBranch(b)])
      .then(([c, x, y]) => { if (!cancelled) { setData(c); setEvs([x.events, y.events]); } })
      .catch((e) => { if (!cancelled) setError(String(e.message ?? e)); });
    return () => { cancelled = true; };
  }, [a, b]);

  return (
    <>
      <header className="page-head"><h1>Compare</h1><span className="spacer" /><span className="muted small">branch {a.slice(0, 8)} vs {b.slice(0, 8)}</span></header>
      <main className="content wide">
        {error && <div className="notice error" role="alert">{error}</div>}
        {!data && !error && <p className="muted">Loading…</p>}
        {data && (
          <>
            <div className="cmp-cols"><Side id={a} s={data.a} /><Side id={b} s={data.b} /></div>
            <div className="panel pane">
              {data.first_divergent_seq != null
                ? <strong>First divergence at step {data.first_divergent_seq}</strong>
                : <strong>The branches never diverge</strong>}
            </div>
            {data.first_divergent_seq != null && evs && (
              <div className="cmp-cols">
                <StepView id={a} ev={evs[0].find((e) => e.seq === data.first_divergent_seq)} />
                <StepView id={b} ev={evs[1].find((e) => e.seq === data.first_divergent_seq)} />
              </div>
            )}
            <div className="panel pane">
              <div className="label">Differing rows ({data.differing_rows.length})</div>
              {data.differing_rows.length === 0 && <p className="muted">The final databases are identical.</p>}
              {data.differing_rows.map((r) => {
                const d = jsonDiff(r.a ?? undefined, r.b ?? undefined);
                return (
                  <div key={r.key} className="row-doc">
                    <div className="mono small">{r.key}{!r.a && <span className="muted"> (only in B)</span>}{!r.b && <span className="muted"> (only in A)</span>}</div>
                    <pre className="json diff">{d.map((l, i) => <div key={i} className={`dl dl${l.op === '+' ? 'add' : l.op === '-' ? 'del' : 'same'}`}>{l.op} {l.text}</div>)}</pre>
                  </div>
                );
              })}
            </div>
          </>
        )}
      </main>
    </>
  );
}
