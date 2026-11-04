import { useEffect, useState } from 'react';
import { defaultTeam } from '../api';
import { ModelHint, ModelInput, ProviderSelect } from '../ProviderFields';
import { MODE_LABEL, providerOf, useProviderList } from '../providers';
import { clearStoredTeam, loadTeam, saveTeam, slug, withSlugIds } from '../team';
import type { AgentKind, AgentSpec, ProviderStatus, Team as TeamT } from '../types';

const KINDS: AgentKind[] = ['claude_code', 'codex', 'open'];

/** Mode, then (API mode) provider and model; a CLI agent's model is the CLI's. */
export function AgentFields({ agent: a, index: i, openOk, list, patch }: {
  agent: AgentSpec;
  index: number;
  /** Some provider can take API-mode calls. */
  openOk: boolean;
  list: ProviderStatus[];
  patch: (p: Partial<AgentSpec>) => void;
}) {
  const provider = a.kind === 'open' ? providerOf(a) : null;
  return (
    <>
      <div className="agent-fields">
        <label className="field mode-field">
          <span className="label">Mode</span>
          <select value={a.kind} onChange={(e) => patch({ kind: e.target.value as AgentKind, model: null, provider: undefined })}>
            {KINDS.map((k) => (
              <option key={k} value={k} disabled={k === 'open' && !openOk && a.kind !== 'open'}>
                {MODE_LABEL[k]}{k === 'open' && !openOk ? ' (no provider available)' : ''}
              </option>
            ))}
          </select>
        </label>
        {provider && <ProviderSelect value={provider} list={list} onChange={(id) => patch({ provider: id, model: null })} />}
        <ModelInput
          listId={`team-models-${i}`}
          provider={provider}
          value={a.model ?? ''}
          list={list}
          onChange={(m) => patch({ model: m.trim() === '' ? null : m })}
        />
      </div>
      {provider && <ModelHint provider={provider} model={a.model} list={list} />}
    </>
  );
}

export default function Team() {
  const { list } = useProviderList();
  const [team, setTeam] = useState<TeamT | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    loadTeam().then((t) => { setTeam(t); setLoaded(true); }).catch((e) => setError(String(e.message ?? e)));
  }, []);

  // Persist every edit once the team has loaded.
  useEffect(() => {
    if (loaded && team) saveTeam(team);
  }, [loaded, team]);

  const setAgents = (fn: (a: AgentSpec[]) => AgentSpec[]) =>
    setTeam((t) => (t ? { ...t, agents: withSlugIds(fn(t.agents)) } : t));
  const patch = (i: number, p: Partial<AgentSpec>) => setAgents((as) => as.map((a, j) => (j === i ? { ...a, ...p } : a)));
  const move = (i: number) =>
    setAgents((as) => {
      const n = [...as];
      [n[i - 1], n[i]] = [n[i], n[i - 1]];
      return n;
    });
  const remove = (i: number) => setAgents((as) => as.filter((_, j) => j !== i));
  const add = () =>
    setAgents((as) => [
      ...as,
      { id: slug('agent', new Set(as.map((a) => a.id))), name: `Agent ${as.length + 1}`, kind: 'claude_code', model: null, prompt: '' },
    ]);
  const reset = async () => {
    clearStoredTeam();
    try {
      setTeam(await defaultTeam());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  // Any provider not known to be unavailable (a local server not checked yet counts).
  const openOk = list.some((s) => s.available || (s.kind === 'local' && !s.checked));

  return (
    <>
      <header className="page-head">
        <h1>Team</h1>
        <span className="muted small">Agents run in order, each handing off to the next; any agent may write unless you make it read-only. New runs use this team.</span>
        <span className="spacer" />
        <button className="btn" onClick={reset}>Reset to default</button>
        <button className="btn primary" onClick={add} disabled={!team}>Add agent</button>
      </header>
      <main className="content">
        {error && <div className="notice error" role="alert">{error}</div>}
        {team?.agents.map((a, i) => (
          <section className="panel agent-card" key={i}>
            <div className="agent-top">
              <span className="step-no">{i + 1}</span>
              <input
                className="grow name-input"
                value={a.name}
                onChange={(e) => patch(i, { name: e.target.value })}
                placeholder="Name"
              />
              <span className="mono muted small" title="Agent id">{a.id}</span>
              <button className="btn btn-sm" onClick={() => move(i)} disabled={i === 0} title="Move up">↑ Up</button>
              <button className="btn btn-sm danger" onClick={() => remove(i)} disabled={team.agents.length === 1} title="Remove">Remove</button>
            </div>
            <AgentFields agent={a} index={i} openOk={openOk} list={list} patch={(p) => patch(i, p)} />
            <label className="check-field" title="Give this agent only the world's read tools">
              <input type="checkbox" checked={!!a.read_only} onChange={(e) => patch(i, { read_only: e.target.checked })} />
              <span>Read-only</span>
            </label>
            <label className="field">
              <span className="label">Prompt</span>
              <textarea value={a.prompt} rows={6} onChange={(e) => patch(i, { prompt: e.target.value })} />
            </label>
          </section>
        ))}
      </main>
    </>
  );
}
