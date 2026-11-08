import { Play } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { startRun } from '../api';
import { useHealth } from '../components';
import { Button } from '../components/ui/button';
import { Field, NativeSelect } from '../components/ui/input';
import { PageBody } from '../components/ui/page';
import { ErrorState } from '../components/ui/state';
import { LiveRun, StartingRun } from '../home/LiveView';
import { missingFor, restoringLine, runLine, taskName } from '../home/summary';
import TeamCard, { TeamActions } from '../home/TeamCard';
import DisruptionPicker from '../home/DisruptionPicker';
import { RUNNER_WORDS, RUNNERS } from '../home/runners';
import { type Task, useSuggestedTeams, useTasks, useTeam, useWorldPreview } from '../home/useHome';
import WorldCard from '../home/WorldCard';
import { useScenario } from '../scenario/ScenarioBuilder';
import { teamForRun } from '../team';
import type { Health, Runner } from '../types';

/** The runner and the disruptions, behind Options. */
function Options({ s, busy }: { s: Setup; busy: boolean }) {
  return (
    <div className="flex w-full flex-col gap-4 rounded-lg bg-surface p-4">
      <DisruptionPicker s={s.scenario} busy={busy} />
      <Field label="How the agents run" className="w-72">
        <NativeSelect value={s.runner} onChange={(e) => s.setRunner(e.target.value as Runner)}>
          {RUNNERS.map((r) => <option key={r.id} value={r.id}>{r.label}</option>)}
        </NativeSelect>
      </Field>
      <div className="flex flex-col gap-1">
        <span className="text-2xs font-medium text-muted">Team</span>
        <TeamActions team={s.team.team} suggested={s.suggested} onImported={s.team.replace} />
      </div>
    </div>
  );
}

/** The one Run button (what will run is its tooltip), what is missing, and Options. */
function RunBar({ s, missing, error, disabled, onRun }: { s: Setup; missing: string[]; error: string | null; disabled: boolean; onRun: () => void }) {
  const [open, setOpen] = useState(false);
  const team = s.team.team;
  const line = team ? `${runLine(team)}${s.runner === 'chain' || team.agents.length < 2 ? '' : ` · ${RUNNER_WORDS[s.runner]}`}` : '';
  return (
    <div className="flex flex-col items-center gap-3">
      <Button variant="primary" className="h-11 gap-2 px-14 text-base [&_svg]:size-4" onClick={onRun} disabled={disabled || missing.length > 0} title={line}><Play />Run</Button>
      {missing.map((m) => <p key={m} className="text-sm font-medium text-alarm" role="status">⚠ {m}</p>)}
      {error && <ErrorState error={error} hint="Nothing was started." className="max-w-2xl" />}
      <button type="button" className="text-xs text-muted hover:text-fg" aria-expanded={open} onClick={() => setOpen(!open)}>{open ? 'Hide options' : 'Options'}</button>
      {open && <Options s={s} busy={false} />}
    </div>
  );
}

/** What a run is started with: the task, the team, how its agents run and the disruptions. */
function useSetup() {
  const tasks = useTasks();
  const team = useTeam();
  const suggested = useSuggestedTeams();
  const [taskId, setTaskId] = useState('');
  const [runner, setRunner] = useState<Runner>('chain');
  // A team comes with the runner its prompts are written for (a suggested team); others run in a chain.
  const teamRunner = team.team?.runner ?? 'chain';
  useEffect(() => { setRunner(teamRunner); }, [team.team?.id, teamRunner]);
  const task: Task | null = tasks.tasks?.find((t) => t.id === taskId) ?? tasks.tasks?.[0] ?? null;
  const scenario = useScenario(true, task ? [task.id] : []);
  const nameOf = (id: string) => { const t = tasks.tasks?.find((x) => x.id === id); return t ? taskName(t) : ''; };
  return { tasks, team, suggested, task, setTaskId, runner, setRunner, scenario, nameOf };
}

type Setup = ReturnType<typeof useSetup>;

/** Starting a run: the world is restored while the request waits, then the address names the run. */
function useStart(s: Setup, open: (run: string, branch: string) => void, back: () => void) {
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const run = async () => {
    const team = s.team.team;
    if (!team || !s.task) return;
    setStarting(true); setError(null);
    try {
      const r = await startRun(s.task.id, teamForRun(team, s.runner), s.runner, s.scenario.scenario);
      open(r.run_id, r.branch_id);
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); back(); }
    setStarting(false);
  };
  return { starting, error, run };
}

/** What must be fixed before the team can run; an external system runs the agents itself, so nothing here has to be installed for it. */
const missingOf = (s: Setup, health: Health | null) => (s.team.team && s.runner !== 'external' ? missingFor(s.team.team, health) : []);

/** Before a run: the world and its task, the team, and the Run button. */
function SetupView({ s, health, start }: { s: Setup; health: Health | null; start: ReturnType<typeof useStart> }) {
  const preview = useWorldPreview();
  const team = s.team.team;
  const missing = missingOf(s, health);
  const loadError = s.tasks.error ?? s.team.error;
  const benchmark = preview.bench?.name ?? health?.benchmark?.name ?? '';
  return (
    <PageBody className="mx-auto w-full max-w-3xl gap-8 py-10">
      {loadError && <ErrorState error={loadError} />}
      <WorldCard benchmark={benchmark} title={preview.twin?.title} tasks={s.tasks.tasks} task={s.task}
        onTask={s.setTaskId} scenario={s.scenario} busy={start.starting} />
      <TeamCard team={team} runner={s.runner} />
      <RunBar s={s} missing={missing} error={start.error} onRun={start.run} disabled={!team || !s.task || !!s.scenario.problem} />
    </PageBody>
  );
}

const partsOf = (h: Health | null) => (h?.benchmark?.world ?? []).map((p) => p.part);

/** Run, the home page: pick the world's task and see the team, press Run, and the same screen becomes the live run. */
export default function Home() {
  const [sp, setSp] = useSearchParams();
  const s = useSetup();
  const health = useHealth();
  const start = useStart(s, (run, branch) => setSp({ run, branch }), () => setSp({}));
  const runId = sp.get('run');
  const branchId = sp.get('branch');
  if (start.starting) return <PageBody><StartingRun title={s.task ? taskName(s.task) : ''} line={restoringLine(partsOf(health))} /></PageBody>;
  if (runId && branchId) {
    return <PageBody><LiveRun key={branchId} runId={runId} branchId={branchId} nameOf={s.nameOf} /></PageBody>;
  }
  return <SetupView s={s} health={health} start={start} />;
}
