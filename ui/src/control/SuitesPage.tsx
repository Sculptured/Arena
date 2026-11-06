import { useCallback, useEffect, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { tasks as fetchTasks } from '../api';
import { formatTime } from '../components';
import { loadTeam, teamForRun } from '../team';
import type { Runner, TaskSummary } from '../types';
import { createSuite, suites as fetchSuites } from './api';
import { StatusMark } from './Marks';
import { suiteMark } from './suite';
import type { ListedSuite } from './types';
import './control.css';

const RUNNERS: Record<Runner, string> = { chain: 'one after another', concurrent: 'all at once', external: 'external system' };

function NewSuiteForm({ tasks, onError }: { tasks: TaskSummary[]; onError: (m: string | null) => void }) {
  const nav = useNavigate();
  const [picked, setPicked] = useState<string[]>([]);
  const [repeats, setRepeats] = useState(2);
  const [runner, setRunner] = useState<Runner>('chain');
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  const toggle = (id: string) => setPicked((p) => (p.includes(id) ? p.filter((x) => x !== id) : [...p, id]));

  const start = async () => {
    setBusy(true);
    onError(null);
    try {
      const team = teamForRun(await loadTeam());
      const chosen = tasks.map((t) => t.id).filter((t) => picked.includes(t));
      const r = await createSuite({ name: name.trim() || undefined, team, tasks: chosen, repeats, runner });
      nav(`/suites/${r.id}`);
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };

  return (
    <div className="panel hmi-form">
      <div className="hmi-form-row">
        <label className="field grow"><span className="label">Name</span>
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="your team's name" disabled={busy} /></label>
        <label className="field"><span className="label">Repeats</span>
          <input type="number" min={1} max={100} value={repeats} onChange={(e) => setRepeats(Number(e.target.value))} disabled={busy} /></label>
        <label className="field"><span className="label">Runs made by</span>
          <select value={runner} onChange={(e) => setRunner(e.target.value as Runner)} disabled={busy}>
            {Object.entries(RUNNERS).map(([k, v]) => <option key={k} value={k}>{v}</option>)}
          </select></label>
        <button className="btn primary" onClick={start} disabled={busy || !tasks.length}>{busy ? 'Starting…' : 'New suite'}</button>
      </div>
      <div className="hmi-tasks">
        <span className="label">Scenarios {picked.length ? `(${picked.length})` : '(all)'}</span>
        {tasks.map((t) => (
          <label key={t.id} className="check-field small">
            <input type="checkbox" checked={picked.includes(t.id)} onChange={() => toggle(t.id)} disabled={busy} />
            <span className="mono">{t.id}</span> <span className="muted">{t.reason}</span>
          </label>
        ))}
      </div>
      <span className="muted small">Runs your <Link to="/team">team</Link> over each scenario on the same seeded world.</span>
    </div>
  );
}

function SuiteRow({ s, picked, onPick }: { s: ListedSuite; picked: boolean; onPick: () => void }) {
  return (
    <tr>
      <td><input type="checkbox" checked={picked} onChange={onPick} aria-label={`compare ${s.name}`} /></td>
      <td><StatusMark mark={suiteMark(s.status)} /></td>
      <td><Link to={`/suites/${s.id}`}>{s.name}</Link></td>
      <td className="muted">{s.team.name}</td>
      <td className="mono">{s.tasks.length} × {s.repeats}</td>
      <td className="muted">{RUNNERS[s.runner]}</td>
      <td className="mono">{s.ended}/{s.runs}</td>
      <td className="mono">{s.passed}</td>
      <td className="muted small">{formatTime(s.created_at)}</td>
    </tr>
  );
}

export default function SuitesPage() {
  const nav = useNavigate();
  const [list, setList] = useState<ListedSuite[] | null>(null);
  const [tasks, setTasks] = useState<TaskSummary[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [picked, setPicked] = useState<string[]>([]);
  const load = useCallback(() => { fetchSuites().then(setList).catch((e) => setError(String(e.message ?? e))); }, []);
  useEffect(() => { load(); fetchTasks().then(setTasks).catch(() => {}); }, [load]);
  const pick = (id: string) => setPicked((p) => (p.includes(id) ? p.filter((x) => x !== id) : [...p.slice(-1), id]));

  return (
    <>
      <header className="page-head"><h1>Test suites</h1><span className="spacer" />
        <button className="btn" disabled={picked.length !== 2} onClick={() => nav(`/suites/compare/${picked[0]}/${picked[1]}`)}>Compare two</button>
      </header>
      <main className="content wide">
        {error && <div className="notice error">{error}</div>}
        <NewSuiteForm tasks={tasks} onError={setError} />
        {list && list.length === 0 && <p className="muted">No suites yet.</p>}
        {list && list.length > 0 && (
          <table className="hmi-table panel">
            <thead><tr><th /><th>Status</th><th>Suite</th><th>Team</th><th>Scenarios × repeats</th><th>Runs made by</th><th>Ended</th><th>Passed</th><th>Created</th></tr></thead>
            <tbody>{list.map((s) => <SuiteRow key={s.id} s={s} picked={picked.includes(s.id)} onPick={() => pick(s.id)} />)}</tbody>
          </table>
        )}
      </main>
    </>
  );
}
