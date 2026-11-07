import { useState } from 'react';
import { fork } from '../api';
import { changedFields } from './timeline';
import { RestoreChoice, blockedReason, useRestorePlan } from './RestoreChoice';
import { ModelHint, ModelInput, ProviderSelect } from '../ProviderFields';
import { Button } from '../components/ui/button';
import { Field, NativeSelect, Textarea } from '../components/ui/input';
import { ErrorState } from '../components/ui/state';
import { MODE_LABEL, modelProblem, providerId, providerOf, useProviderList } from '../providers';
import type { AgentKind, AgentSpec, BranchDetail, Change, ForkResponse, ProviderStatus, RestoreMode } from '../types';

const MODES: AgentKind[] = ['claude_code', 'codex', 'open'];

/** The agent the fork would run: `c` applied as arenad's `fork::apply_change` does. */
export function applied(a: AgentSpec, c: Change): AgentSpec {
  const kindChanged = c.kind != null && c.kind !== a.kind;
  const providerChanged = c.provider != null && providerId(c.provider) !== providerOf(a);
  // The old model belongs to the old mode or provider: the new one's default applies.
  let model = kindChanged || providerChanged ? null : a.model;
  if (c.model != null) model = c.model.trim() || null;
  return {
    ...a,
    prompt: c.prompt ?? a.prompt,
    kind: c.kind ?? a.kind,
    provider: c.provider != null ? c.provider : a.provider,
    model,
  };
}

/** Why the fork can't run a changed API-mode agent (the provider is unavailable, or a model is needed). */
export function forkProblem(agents: AgentSpec[], changes: Record<string, Change>, list: ProviderStatus[]): string | null {
  for (const a of agents) {
    const c = changes[a.id];
    if (!c) continue;
    const n = applied(a, c);
    if (n.kind !== 'open') continue;
    const p = modelProblem(providerOf(n), n.model, list);
    if (p) return `${a.name}: ${p}`;
  }
  return null;
}

export function AgentEdit({ agent: a, edit: e, list, onEdit }: {
  agent: AgentSpec;
  edit: Change;
  list: ProviderStatus[];
  onEdit: (patch: Change) => void;
}) {
  const kind = e.kind ?? a.kind;
  const provider = kind === 'open' ? providerId(e.provider ?? a.provider) : null;
  const model = e.model ?? a.model ?? '';
  return (
    <div className="flex flex-col gap-2 border-t border-line px-3 py-2.5">
      <div className="flex flex-wrap items-end gap-2">
        <strong className="w-full text-sm">{a.name}</strong>
        <Field label="Mode" className="w-36">
          <NativeSelect value={kind} onChange={(ev) => onEdit({ kind: ev.target.value as AgentKind, model: '', provider: undefined })}>
            {MODES.map((m) => <option key={m} value={m}>{MODE_LABEL[m]}</option>)}
          </NativeSelect>
        </Field>
        {provider && (
          <ProviderSelect value={provider} list={list} onChange={(id) => onEdit({ provider: id, model: '' })} />
        )}
        <ModelInput listId={`fork-models-${a.id}`} provider={provider} value={model} list={list} onChange={(m) => onEdit({ model: m })} />
      </div>
      {provider && <ModelHint provider={provider} model={model} list={list} />}
      <Field label="Prompt">
        <Textarea rows={4} value={e.prompt ?? a.prompt} onChange={(ev) => onEdit({ prompt: ev.target.value })} />
      </Field>
    </div>
  );
}

/** The start button; `blocked` disables it and says why. */
export function ForkActions({ blocked, busy, changes: n, onGo }: { blocked: string | null; busy: boolean; changes: number; onGo: () => void }) {
  return (
    <div className="flex flex-col gap-1.5">
      <Button variant="primary" className="self-start" onClick={onGo} disabled={busy || blocked != null}>
        {busy ? 'Forking…' : `Change and rerun from here${n ? ` (${n} change${n === 1 ? '' : 's'})` : ''}`}
      </Button>
      {blocked && <span className="text-xs text-muted" role="status">{blocked}</span>}
    </div>
  );
}

/** Edits to the team, the restore and the start button, kept for one step. */
function useFork(branch: BranchDetail, seq: number, onForked: (r: ForkResponse) => void) {
  const [edits, setEdits] = useState<Record<string, Change>>({});
  const [restore, setRestore] = useState<RestoreMode>('exact');
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const plan = useRestorePlan(branch.id, seq, restore);
  const { list } = useProviderList();
  const set = (id: string, patch: Change) => setEdits((e) => ({ ...e, [id]: { ...e[id], ...patch } }));
  const changes = changedFields(branch.team.agents, edits);
  const go = async () => {
    setBusy(true); setErr(null);
    try { onForked(await fork(branch.id, seq, changes, restore)); } catch (e) { setErr(e instanceof Error ? e.message : String(e)); setBusy(false); }
  };
  const blocked = blockedReason(plan, branch.status === 'running') ?? forkProblem(branch.team.agents, changes, list);
  return { edits, set, restore, setRestore, plan, list, changes, go, busy, err, blocked };
}

export default function ForkPanel({ branch, seq, onForked, onClose }: {
  branch: BranchDetail;
  seq: number;
  onForked: (r: ForkResponse) => void;
  onClose?: () => void;
}) {
  const f = useFork(branch, seq, onForked);
  return (
    <div className="flex flex-col">
      <div className="flex items-start gap-2 px-3 py-2">
        <div className="min-w-0 flex-1">
          <h3 className="text-sm font-semibold">Change and rerun from step {seq}</h3>
          <p className="text-xs text-muted">Only fields you edit are changed. Everything before this step is reused.</p>
        </div>
        {onClose && <Button size="sm" variant="ghost" onClick={onClose}>Close</Button>}
      </div>
      {branch.team.agents.map((a) => <AgentEdit key={a.id} agent={a} edit={f.edits[a.id] ?? {}} list={f.list} onEdit={(p) => f.set(a.id, p)} />)}
      <div className="flex flex-col gap-3 border-t border-line px-3 py-2.5">
        <RestoreChoice mode={f.restore} onMode={f.setRestore} plan={f.plan} />
        {f.err && <ErrorState error={f.err} hint="Nothing was started. Fix the cause and try again." />}
        <ForkActions blocked={f.blocked} busy={f.busy} changes={Object.keys(f.changes).length} onGo={f.go} />
      </div>
    </div>
  );
}
