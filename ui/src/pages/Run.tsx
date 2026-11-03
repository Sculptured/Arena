import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useLocation, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { branch as fetchBranch, runs as fetchRuns, stream } from '../api';
import { ApproximateMark, StatusChip } from '../components';
import ForkPanel from '../run/ForkPanel';
import Inspector from '../run/Inspector';
import Timeline from '../run/Timeline';
import { formatCost, lanes, summarize } from '../run/timeline';
import type { Branch, BranchDetail, Event } from '../types';
import { worldLine } from '../restore';
import { costIsUnknown, teamProviders } from '../providers';
import { branchTree } from './Runs';

export default function Run() {
  const { runId = '', branchId = '' } = useParams();
  const nav = useNavigate();
  const loc = useLocation();
  const [sp] = useSearchParams();
  const [detail, setDetail] = useState<BranchDetail | null>(null);
  const [siblings, setSiblings] = useState<Branch[]>([]);
  const [selected, setSelected] = useState<number | null>(sp.get('step') ? Number(sp.get('step')) : null);
  const [forking, setForking] = useState(sp.get('fork') === '1');
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>((loc.state as { notice?: string } | null)?.notice ?? null);
  const [picking, setPicking] = useState(false);

  const current = useRef(branchId);
  current.current = branchId;
  // A response only lands if it is still for the branch on screen.
  const load = useCallback(() => {
    const id = branchId;
    fetchBranch(id).then((d) => { if (current.current === id) { setDetail(d); setError(null); } })
      .catch((e) => { if (current.current === id) setError(String(e.message ?? e)); });
    fetchRuns().then((r) => { if (current.current === id) setSiblings(r.find((x) => x.id === runId)?.branches ?? []); }).catch(() => {});
  }, [branchId, runId]);

  useEffect(() => {
    setDetail(null); setSelected(sp.get('step') ? Number(sp.get('step')) : null); setForking(sp.get('fork') === '1'); setPicking(false);
    setNotice((loc.state as { notice?: string } | null)?.notice ?? null);
    load();
  }, [load]); // eslint-disable-line react-hooks/exhaustive-deps

  const running = detail?.status === 'running';
  useEffect(() => {
    if (!running) return;
    const off = stream(
      branchId,
      (e: Event) => setDetail((d) => (d && d.id === branchId && !d.events.some((x) => x.seq === e.seq) ? { ...d, events: [...d.events, e] } : d)),
      () => load(),
    );
    return off;
  }, [running, branchId, load]);

  const ls = useMemo(() => (detail ? lanes(detail.events, detail.team, detail.start_seq) : []), [detail]);
  const order = useMemo(() => ls.flatMap((l) => l.steps).map((s) => s.seq).sort((a, b) => a - b), [ls]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (/^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName) || e.metaKey || e.ctrlKey || e.altKey) return;
      const dir = e.key === 'ArrowDown' || e.key === 'j' ? 1 : e.key === 'ArrowUp' || e.key === 'k' ? -1 : 0;
      if (!dir || !order.length) return;
      e.preventDefault();
      setSelected((s) => {
        const i = s == null ? (dir > 0 ? -1 : order.length) : order.indexOf(s);
        return order[Math.min(order.length - 1, Math.max(0, i + dir))];
      });
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [order]);

  if (!detail) {
    return (
      <>
        <header className="page-head"><h1>Run</h1></header>
        <main className="content">{error ? <div className="notice error">{error}</div> : <p className="muted">Loading…</p>}</main>
      </>
    );
  }

  const sm = summarize(detail, detail.events);
  const taskId = (detail.events.find((e) => e.kind === 'run_start')?.payload as any)?.task_id ?? '';
  const event = detail.events.find((e) => e.seq === selected) ?? null;
  const world = worldLine(detail.events);
  const others = siblings.filter((b) => b.id !== detail.id && (b.status === 'passed' || b.status === 'failed'));

  return (
    <>
      <header className="page-head run-header">
        <h1 className="mono">{taskId ? `task ${taskId}` : 'Run'}</h1>
        <StatusChip status={detail.status} />
        <ApproximateMark of={detail} />
        <span className="muted mono small">{detail.id.slice(0, 8)}</span>
        <span className="spacer" />
        {sm.scope && <span className="muted small">{sm.scope}</span>}
        <span className="stat"><span className="label">Seconds{sm.live ? ' so far' : ''}</span>{sm.seconds != null ? sm.seconds.toFixed(1) : '–'}</span>
        <span className="stat"><span className="label">Tokens</span>{sm.tokens}</span>
        <span className="stat"><span className="label">Cost</span>{formatCost(sm.cost_usd, sm.cost_source, teamProviders(detail.team).some(costIsUnknown))}</span>
      </header>
      <div className="run-body">
        <section className="run-main">
          {notice && <div className="notice info" role="status">{notice} <button className="btn btn-sm" onClick={() => setNotice(null)}>Dismiss</button></div>}
          {error && <div className="notice error" role="alert">{error}</div>}
          {detail.error && <div className="notice error">{detail.error}</div>}
          {detail.fork_note && <div className="muted small">{detail.fork_note}</div>}
          {world && <div className="muted small world-line" title={world}>{world}</div>}
          <div className="panel pane">
            <Timeline lanes={ls} team={detail.team} selected={selected} onSelect={(s) => { setSelected(s); }} />
          </div>
          <div className="panel pane">
            <Inspector branchId={detail.id} event={event} team={detail.team} autoRows={sp.get('rows') === '1'} />
            {event && (
              <div className="insp-actions">
                <button className="btn" onClick={() => setForking((f) => !f)} disabled={event.kind === 'check' || event.kind === 'run_start'}>
                  Change and rerun from here
                </button>
              </div>
            )}
          </div>
          {forking && selected != null && (
            <ForkPanel
              key={`${detail.id}:${selected}`}
              branch={detail}
              seq={selected}
              onForked={(r) => nav(`/runs/${runId}/${r.branch_id}`, {
                state: { notice: r.moved_from != null ? `Restarted from step ${r.start_seq} instead of ${r.moved_from}: ${r.reason ?? ''}` : null },
              })}
            />
          )}
        </section>
        <aside className="run-side panel">
          <div className="label side-title">Branches</div>
          <ul className="tree">
            {branchTree(siblings).map(({ branch: b, depth }) => (
              <li key={b.id}>
                <button className={`branch-row${b.id === detail.id ? ' current' : ''}`} style={{ paddingLeft: 8 + depth * 16 }}
                  onClick={() => nav(`/runs/${runId}/${b.id}`)}>
                  <span className="tree-mark" aria-hidden="true">{depth ? '└' : '●'}</span>
                  <StatusChip status={b.status} />
                  <span className="mono branch-id">{b.id.slice(0, 6)}</span>
                  <ApproximateMark of={b} />
                </button>
              </li>
            ))}
          </ul>
          <div className="side-actions">
            <button className="btn btn-sm" onClick={() => setPicking((p) => !p)} disabled={detail.status === 'running' || others.length === 0}>Compare with…</button>
            {picking && others.map((b) => (
              <button key={b.id} className="btn btn-sm" onClick={() => nav(`/compare/${detail.id}/${b.id}`)}>
                <span className="mono">{b.id.slice(0, 6)}</span> {b.status}
              </button>
            ))}
            {others.length === 0 && <span className="muted small">No other finished branches yet.</span>}
          </div>
        </aside>
      </div>
    </>
  );
}
