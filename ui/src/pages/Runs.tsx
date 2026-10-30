import { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { runs as fetchRuns, startRun, tasks as fetchTasks } from '../api';
import { StatusChip, formatTime } from '../components';
import { loadTeam } from '../team';
import type { Branch, RunSummary, TaskSummary } from '../types';

/** Branches in tree order with their depth (parent before children, oldest first). */
export function branchTree(branches: Branch[]): { branch: Branch; depth: number }[] {
  const ids = new Set(branches.map((b) => b.id));
  const kids = new Map<string | null, Branch[]>();
  for (const b of [...branches].sort((x, y) => x.created_at.localeCompare(y.created_at))) {
    const key = b.parent_id && ids.has(b.parent_id) ? b.parent_id : null;
    kids.set(key, [...(kids.get(key) ?? []), b]);
  }
  const out: { branch: Branch; depth: number }[] = [];
  const walk = (key: string | null, depth: number) => {
    for (const b of kids.get(key) ?? []) {
      out.push({ branch: b, depth });
      walk(b.id, depth + 1);
    }
  };
  walk(null, 0);
  return out;
}

function NewRun({ tasks, onError }: { tasks: TaskSummary[]; onError: (m: string | null) => void }) {
  const nav = useNavigate();
  const [taskId, setTaskId] = useState('');
  const [busy, setBusy] = useState(false);
  const selected = taskId || tasks[0]?.id || '';

  const start = async () => {
    if (!selected) return;
    setBusy(true);
    onError(null);
    try {
      const team = await loadTeam();
      const r = await startRun(selected, team);
      nav(`/runs/${r.run_id}/${r.branch_id}`);
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };

  return (
    <div className="panel new-run">
      <label className="field grow">
        <span className="label">Task</span>
        <select value={selected} onChange={(e) => setTaskId(e.target.value)} disabled={busy}>
          {tasks.map((t) => (
            <option key={t.id} value={t.id}>{t.id} · {t.reason}</option>
          ))}
        </select>
      </label>
      <button className="btn primary" onClick={start} disabled={busy || !selected}>
        {busy ? 'Starting…' : 'New run'}
      </button>
      <span className="muted small">uses your <a href="/team" onClick={(e) => { e.preventDefault(); nav('/team'); }}>team</a></span>
    </div>
  );
}

export default function Runs() {
  const nav = useNavigate();
  const [runs, setRuns] = useState<RunSummary[] | null>(null);
  const [tasks, setTasks] = useState<TaskSummary[]>([]);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    fetchRuns().then((r) => { setRuns(r); setError(null); }).catch((e) => setError(String(e.message ?? e)));
  }, []);

  useEffect(() => {
    refresh();
    fetchTasks().then(setTasks).catch((e) => setError(String(e.message ?? e)));
  }, [refresh]);

  const anyRunning = useMemo(() => !!runs?.some((r) => r.branches.some((b) => b.status === 'running')), [runs]);
  useEffect(() => {
    if (!anyRunning) return;
    const t = setInterval(refresh, 3000);
    return () => clearInterval(t);
  }, [anyRunning, refresh]);

  return (
    <>
      <header className="page-head">
        <h1>Runs</h1>
        <span className="muted small">{runs ? `${runs.length} run${runs.length === 1 ? '' : 's'}` : ''}</span>
      </header>
      <main className="content">
        <NewRun tasks={tasks} onError={setError} />
        {error && <div className="notice error" role="alert">{error}</div>}
        {runs && runs.length === 0 && (
          <div className="empty">
            <h2>No runs yet</h2>
            <p className="muted">Pick a task above and start a run. Every step is recorded, and you can fork from any of them.</p>
          </div>
        )}
        {runs?.map((run) => (
          <section className="panel run" key={run.id}>
            <div className="run-head">
              <span className="mono run-task">{run.task_id}</span>
              <span className="muted small">{formatTime(run.created_at)}</span>
              <span className="muted small mono">{run.id.slice(0, 8)}</span>
            </div>
            <ul className="tree">
              {branchTree(run.branches).map(({ branch: b, depth }) => (
                <li key={b.id}>
                  <button
                    className="branch-row"
                    style={{ paddingLeft: 12 + depth * 22 }}
                    onClick={() => nav(`/runs/${run.id}/${b.id}`)}
                  >
                    <span className="tree-mark" aria-hidden="true">{depth ? '└' : '●'}</span>
                    <StatusChip status={b.status} />
                    <span className="mono branch-id">{b.id.slice(0, 8)}</span>
                    {b.fork_note && <span className="fork-note">{b.fork_note}</span>}
                    <span className="spacer" />
                    <span className="muted small">{formatTime(b.created_at)}</span>
                  </button>
                </li>
              ))}
            </ul>
          </section>
        ))}
      </main>
    </>
  );
}
