import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Link, useParams } from 'react-router-dom';
import { branch as fetchBranch, stream } from '../api';
import { StatusChip } from '../components';
import Inspector from '../run/Inspector';
import type { BranchDetail, Event } from '../types';
import ActionTimeline from './ActionTimeline';
import { actionsOf, worldInfo } from './actions';
import { buildLanes, spanOf } from './lanes';
import { processView } from './process';
import ProcessDiagram from './ProcessDiagram';
import { actionsPerBucket, modelCalls } from './trends';
import TrendsPanel from './TrendsPanel';
import './control.css';

/** Adds a streamed event to the branch on screen, once. */
function withEvent(d: BranchDetail | null, id: string, e: Event): BranchDetail | null {
  if (!d || d.id !== id || d.events.some((x) => x.seq === e.seq)) return d;
  return { ...d, events: [...d.events, e] };
}

/** The run as a process (L2), its action timeline and trends, and one step's trace (L4). Live while it runs. */
export default function ProcessPage() {
  const { runId = '', branchId = '' } = useParams();
  const [d, setD] = useState<BranchDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const current = useRef(branchId);
  current.current = branchId;
  const load = useCallback(() => {
    const id = branchId;
    fetchBranch(id).then((v) => { if (current.current === id) { setD(v); setError(null); } })
      .catch((e) => current.current === id && setError(String(e.message ?? e)));
  }, [branchId]);
  useEffect(() => { setD(null); setSelected(null); load(); }, [load]);
  const running = d?.status === 'running';
  useEffect(() => {
    if (!running) return;
    return stream(branchId, (e) => setD((x) => withEvent(x, branchId, e)), () => load());
  }, [running, branchId, load]);

  const events = useMemo(() => d?.events ?? [], [d]);
  const actions = useMemo(() => actionsOf(events, d?.start_seq ?? null), [events, d]);
  const lanes = useMemo(() => (d ? buildLanes(events, d.team, d.start_seq) : []), [events, d]);
  if (!d) return <><header className="page-head"><h1>Process</h1></header><main className="content">{error ? <div className="notice error">{error}</div> : <p className="muted">Loading…</p>}</main></>;

  const info = worldInfo(events);
  const task = (events.find((e) => e.kind === 'run_start')?.payload as { task_id?: string } | undefined)?.task_id ?? '';
  const event = events.find((e) => e.seq === selected) ?? null;
  return (
    <>
      <header className="page-head">
        <h1 className="mono">{task ? `task ${task}` : 'Run'}</h1><StatusChip status={d.status} />
        <span className="muted mono small">{d.id.slice(0, 8)}</span>
        {!info.known && <span className="muted small">recorded before Arena kept each tool's system and effect: shown as unknown</span>}
        <span className="spacer" /><Link className="btn btn-sm" to={`/runs/${runId}/${branchId}`}>Run page</Link>
      </header>
      <main className="content wide hmi">
        {error && <div className="notice error">{error}</div>}
        {d.error && <div className="notice error">⚠ {d.error}</div>}
        <h2 className="hmi-h2">Process</h2>
        <div className="panel pane"><ProcessDiagram view={processView(events, actions, d.team)} /></div>
        <h2 className="hmi-h2">Actions{running ? ' (live)' : ''}</h2>
        <div className="panel pane"><ActionTimeline lanes={lanes} span={spanOf(lanes, running ? Date.now() : undefined)} selected={selected} onSelect={setSelected} /></div>
        <h2 className="hmi-h2">Trends</h2>
        <div className="panel pane"><TrendsPanel perMinute={actionsPerBucket(actions)} calls={modelCalls(events)} /></div>
        <h2 className="hmi-h2">Step</h2>
        <div className="panel pane"><Inspector branchId={d.id} event={event} team={d.team} /></div>
      </main>
    </>
  );
}
