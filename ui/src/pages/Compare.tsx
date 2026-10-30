import { useEffect, useState } from 'react';
import { useParams } from 'react-router-dom';
import { compare } from '../api';
import { StatusChip } from '../components';
import { formatCost, jsonDiff } from '../run/timeline';
import type { CompareResponse, CompareSide } from '../types';

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
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setData(null); setError(null);
    compare(a, b).then(setData).catch((e) => setError(String(e.message ?? e)));
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
