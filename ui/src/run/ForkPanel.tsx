import { useState } from 'react';
import { fork } from '../api';
import { changedFields } from './timeline';
import type { AgentKind, BranchDetail, Change, ForkResponse } from '../types';

const MODES: { id: AgentKind; label: string }[] = [
  { id: 'claude_code', label: 'Claude Code' },
  { id: 'codex', label: 'Codex' },
  { id: 'open', label: 'Open' },
];

export default function ForkPanel({ branch, seq, onForked }: {
  branch: BranchDetail;
  seq: number;
  onForked: (r: ForkResponse) => void;
}) {
  const [edits, setEdits] = useState<Record<string, Change>>({});
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const set = (id: string, patch: Change) => setEdits((e) => ({ ...e, [id]: { ...e[id], ...patch } }));
  const changes = () => changedFields(branch.team.agents, edits);
  const n = Object.keys(changes()).length;

  const go = async () => {
    setBusy(true); setErr(null);
    try { onForked(await fork(branch.id, seq, changes())); } catch (e) { setErr(e instanceof Error ? e.message : String(e)); setBusy(false); }
  };

  return (
    <div className="fork-panel panel">
      <div className="fork-head">
        <h2>Change and rerun from step {seq}</h2>
        <span className="muted small">Only fields you edit are changed. Everything before this step is reused.</span>
      </div>
      {branch.team.agents.map((a) => {
        const e = edits[a.id] ?? {};
        return (
          <div className="fork-agent" key={a.id}>
            <div className="fork-agent-top">
              <strong>{a.name}</strong>
              <label className="field fork-mode">
                <span className="label">Mode</span>
                <select value={e.kind ?? a.kind} onChange={(ev) => set(a.id, { kind: ev.target.value as AgentKind })}>
                  {MODES.map((m) => <option key={m.id} value={m.id}>{m.label}</option>)}
                </select>
              </label>
              <label className="field grow">
                <span className="label">Model</span>
                <input value={e.model ?? a.model ?? ''} placeholder="default" onChange={(ev) => set(a.id, { model: ev.target.value })} />
              </label>
            </div>
            <label className="field">
              <span className="label">Prompt</span>
              <textarea rows={4} value={e.prompt ?? a.prompt} onChange={(ev) => set(a.id, { prompt: ev.target.value })} />
            </label>
          </div>
        );
      })}
      {err && <div className="notice error" role="alert">{err}</div>}
      <div className="fork-actions">
        <button className="btn primary" onClick={go} disabled={busy || branch.status === 'running'}>
          {busy ? 'Forking…' : `Change and rerun from here${n ? ` (${n} change${n === 1 ? '' : 's'})` : ''}`}
        </button>
        {branch.status === 'running' && <span className="muted small">Wait for this run to finish before forking.</span>}
      </div>
    </div>
  );
}
