import { useEffect, useState } from 'react';
import { defaultTeam } from '../api';
import { useHealth } from '../components';
import { clearStoredTeam, loadTeam, saveTeam, slug, withSlugIds } from '../team';
import type { AgentKind, AgentSpec, Team as TeamT } from '../types';

const KINDS: { value: AgentKind; label: string }[] = [
  { value: 'claude_code', label: 'Claude Code' },
  { value: 'codex', label: 'Codex' },
  { value: 'open', label: 'Open' },
];

export default function Team() {
  const health = useHealth();
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

  const openOk = health?.open_available ?? false;

  return (
    <>
      <header className="page-head">
        <h1>Team</h1>
        <span className="muted small">A chain of agents; the last one writes. New runs use this team.</span>
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
            <div className="agent-fields">
              <label className="field">
                <span className="label">Mode</span>
                <select value={a.kind} onChange={(e) => patch(i, { kind: e.target.value as AgentKind })}>
                  {KINDS.map((k) => (
                    <option key={k.value} value={k.value} disabled={k.value === 'open' && !openOk && a.kind !== 'open'}>
                      {k.label}{k.value === 'open' && !openOk ? ' (needs OPENROUTER_API_KEY)' : ''}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field grow">
                <span className="label">Model</span>
                <input
                  className="mono"
                  value={a.model ?? ''}
                  onChange={(e) => patch(i, { model: e.target.value.trim() === '' ? null : e.target.value })}
                  placeholder="default"
                />
              </label>
            </div>
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
