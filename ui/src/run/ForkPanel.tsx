import { useState } from 'react';
import { fork } from '../api';
import { changedFields } from './timeline';
import { RestoreChoice, blockedReason, useRestorePlan } from './RestoreChoice';
import { DEFAULT_OPENROUTER_MODEL } from '../types';
import type { AgentKind, AgentSpec, BranchDetail, Change, ForkResponse, RestoreMode } from '../types';

const MODES: { id: AgentKind; label: string }[] = [
  { id: 'claude_code', label: 'Claude Code' },
  { id: 'codex', label: 'Codex' },
  { id: 'open', label: 'OpenRouter' },
];

function AgentEdit({ agent: a, edit: e, onEdit }: { agent: AgentSpec; edit: Change; onEdit: (patch: Change) => void }) {
  return (
    <div className="fork-agent">
      <div className="fork-agent-top">
        <strong>{a.name}</strong>
        <label className="field fork-mode">
          <span className="label">Mode</span>
          <select value={e.kind ?? a.kind} onChange={(ev) => onEdit({ kind: ev.target.value as AgentKind, model: '' })}>
            {MODES.map((m) => <option key={m.id} value={m.id}>{m.label}</option>)}
          </select>
        </label>
        <label className="field grow">
          <span className="label">Model</span>
          <input value={e.model ?? a.model ?? ''} placeholder={(e.kind ?? a.kind) === 'open' ? `${DEFAULT_OPENROUTER_MODEL} (default)` : 'default'} onChange={(ev) => onEdit({ model: ev.target.value })} />
        </label>
      </div>
      <label className="field">
        <span className="label">Prompt</span>
        <textarea rows={4} value={e.prompt ?? a.prompt} onChange={(ev) => onEdit({ prompt: ev.target.value })} />
      </label>
    </div>
  );
}

/** The start button; `blocked` disables it and says why. */
export function ForkActions({ blocked, busy, changes: n, onGo }: { blocked: string | null; busy: boolean; changes: number; onGo: () => void }) {
  return (
    <div className="fork-actions">
      <button className="btn primary" onClick={onGo} disabled={busy || blocked != null}>
        {busy ? 'Forking…' : `Change and rerun from here${n ? ` (${n} change${n === 1 ? '' : 's'})` : ''}`}
      </button>
      {blocked && <span className="small" role="status">{blocked}</span>}
    </div>
  );
}

export default function ForkPanel({ branch, seq, onForked }: {
  branch: BranchDetail;
  seq: number;
  onForked: (r: ForkResponse) => void;
}) {
  const [edits, setEdits] = useState<Record<string, Change>>({});
  const [restore, setRestore] = useState<RestoreMode>('exact');
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const plan = useRestorePlan(branch.id, seq, restore);

  const set = (id: string, patch: Change) => setEdits((e) => ({ ...e, [id]: { ...e[id], ...patch } }));
  const changes = () => changedFields(branch.team.agents, edits);

  const go = async () => {
    setBusy(true); setErr(null);
    try { onForked(await fork(branch.id, seq, changes(), restore)); } catch (e) { setErr(e instanceof Error ? e.message : String(e)); setBusy(false); }
  };

  return (
    <div className="fork-panel panel">
      <div className="fork-head">
        <h2>Change and rerun from step {seq}</h2>
        <span className="muted small">Only fields you edit are changed. Everything before this step is reused.</span>
      </div>
      {branch.team.agents.map((a) => <AgentEdit key={a.id} agent={a} edit={edits[a.id] ?? {}} onEdit={(p) => set(a.id, p)} />)}
      <RestoreChoice mode={restore} onMode={setRestore} plan={plan} />
      {err && <div className="notice error" role="alert">{err}</div>}
      <ForkActions blocked={blockedReason(plan, branch.status === 'running')} busy={busy} changes={Object.keys(changes()).length} onGo={go} />
    </div>
  );
}
