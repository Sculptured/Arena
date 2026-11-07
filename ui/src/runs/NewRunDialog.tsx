import { useEffect, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { startRun, tasks as fetchTasks } from '../api';
import { Dialog, DialogActions, DialogContent } from '../components/ui/dialog';
import { Field, NativeSelect } from '../components/ui/input';
import { ErrorState } from '../components/ui/state';
import { agentModelLine } from '../providers';
import ScenarioBuilder, { useScenario } from '../scenario/ScenarioBuilder';
import { loadTeam, teamForRun } from '../team';
import type { Runner, TaskSummary, Team } from '../types';

const RUNNER_HINT = 'Arena tracks every agent either way and imposes no order or handoff. With an external system, the run waits for your own code (or a scripted driver) to drive its agents through their endpoints.';

/** The tasks and the team a new run would use. */
function useRunInputs(open: boolean) {
  const [tasks, setTasks] = useState<TaskSummary[] | null>(null);
  const [team, setTeam] = useState<Team | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!open) return;
    fetchTasks().then(setTasks).catch((e) => setError(String(e.message ?? e)));
    loadTeam().then(setTeam).catch((e) => setError(String(e.message ?? e)));
  }, [open]);
  return { tasks, team, error, setError };
}

function TeamSummary({ team }: { team: Team | null }) {
  if (!team) return <p className="text-xs text-muted">Loading your team…</p>;
  return (
    <div className="rounded-md border border-line bg-raised">
      <div className="flex items-center justify-between border-b border-line px-2.5 py-1 text-2xs font-medium tracking-wide text-muted uppercase">
        <span>Team · {team.agents.length} agents</span><Link to="/team" className="normal-case text-accent hover:underline">Edit team</Link>
      </div>
      <ul className="max-h-40 overflow-auto px-2.5 py-1.5 text-xs">
        {team.agents.map((a) => (
          <li key={a.id} className="flex gap-2 py-0.5"><span className="font-medium">{a.name}</span><span className="truncate font-mono text-2xs text-muted">{agentModelLine(a)}{a.read_only ? ' · read-only' : ''}</span></li>
        ))}
      </ul>
    </div>
  );
}

function TaskField({ tasks, value, onChange, busy }: { tasks: TaskSummary[] | null; value: string; onChange: (id: string) => void; busy: boolean }) {
  return (
    <Field label="Task">
      <NativeSelect value={value} onChange={(e) => onChange(e.target.value)} disabled={busy || !tasks?.length}>
        {!tasks && <option>Loading tasks…</option>}
        {tasks?.map((t) => <option key={t.id} value={t.id}>{t.id} · {t.reason}</option>)}
      </NativeSelect>
    </Field>
  );
}

/** The run's task, rules, disruptions and the start. */
function useNewRun(open: boolean) {
  const nav = useNavigate();
  const inputs = useRunInputs(open);
  const [taskId, setTaskId] = useState('');
  const [runner, setRunner] = useState<Runner>('chain');
  const [busy, setBusy] = useState(false);
  const selected = taskId || inputs.tasks?.[0]?.id || '';
  const scenario = useScenario(open, [selected]);
  const start = async () => {
    setBusy(true); inputs.setError(null);
    try {
      const r = await startRun(selected, teamForRun(inputs.team ?? await loadTeam(), runner), runner, scenario.scenario);
      nav(`/runs/${r.run_id}/${r.branch_id}`);
    } catch (e) { inputs.setError(e instanceof Error ? e.message : String(e)); }
    setBusy(false);
  };
  return { inputs, selected, setTaskId, runner, setRunner, busy, scenario, start };
}

/** Start a run of one task with your team; opens the run when it starts. */
export default function NewRunDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  const { inputs, selected, setTaskId, runner, setRunner, busy, scenario, start } = useNewRun(open);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title="New run" description="Every step is recorded and saved; you can change and rerun from any of them.">
        <div className="flex flex-col gap-3 overflow-auto p-4">
          <TaskField tasks={inputs.tasks} value={selected} onChange={setTaskId} busy={busy} />
          <Field label="Agents run" hint={RUNNER_HINT}>
            <NativeSelect value={runner} onChange={(e) => setRunner(e.target.value as Runner)} disabled={busy}>
              <option value="chain">one after another</option>
              <option value="concurrent">all at once</option>
              <option value="external">by an external system</option>
            </NativeSelect>
          </Field>
          <ScenarioBuilder s={scenario} busy={busy} />
          <TeamSummary team={inputs.team && teamForRun(inputs.team, runner)} />
          {inputs.error && <ErrorState error={inputs.error} hint="Nothing was started. Fix the cause and start again." />}
        </div>
        <DialogActions onCancel={() => onOpenChange(false)} onGo={start} disabled={busy || !selected || !!scenario.problem} label={busy ? 'Starting…' : 'Start run'} />
      </DialogContent>
    </Dialog>
  );
}
