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
import TeamCard from '../home/TeamCard';
import { RUNNER_WORDS, RUNNERS } from '../home/runners';
import { type Task, useSuggestedTeams, useTasks, useTeam, useWorldPreview } from '../home/useHome';
import WorldCard from '../home/WorldCard';
import { useScenario } from '../scenario/ScenarioBuilder';
import { teamForRun } from '../team';
import type { Health, Runner, Team } from '../types';

/** The big Run button, the one line on what will run, what is missing, and the runner under Advanced. */
function RunBar({ team, runner, setRunner, missing, error, disabled, onRun }: {
  team: Team | null; runner: Runner; setRunner: (r: Runner) => void; missing: string[]; error: string | null; disabled: boolean; onRun: () => void;
}) {
  return (
    <div className="flex flex-col items-center gap-2 py-2">
      <Button variant="primary" className="h-11 gap-2 px-12 text-base [&_svg]:size-4" onClick={onRun} disabled={disabled || missing.length > 0}><Play />Run</Button>
      <p className="text-sm text-muted">{team ? `${runLine(team)}${runner === 'chain' || team.agents.length < 2 ? '' : ` · ${RUNNER_WORDS[runner]}`}` : 'Loading your team…'}</p>
      {missing.map((m) => <p key={m} className="text-sm font-medium text-alarm" role="status">⚠ {m}</p>)}
      {error && <ErrorState error={error} hint="Nothing was started. Fix the cause and press Run again." className="max-w-2xl" />}
      <details className="text-sm text-muted">
        <summary className="cursor-pointer text-center text-xs hover:text-fg">Advanced</summary>
        <Field label="How the agents run" className="mt-2 w-72" hint="A team the benchmark suggests picks its own. Arena records every agent either way.">
          <NativeSelect value={runner} onChange={(e) => setRunner(e.target.value as Runner)}>
            {RUNNERS.map((r) => <option key={r.id} value={r.id}>{r.label}</option>)}
          </NativeSelect>
        </Field>
      </details>
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

/** Before a run: the world on the left, the team on the right, the Run button under them. */
function SetupView({ s, health, start }: { s: Setup; health: Health | null; start: ReturnType<typeof useStart> }) {
  const preview = useWorldPreview();
  const team = s.team.team;
  // An external system runs the agents itself: nothing here has to be installed for it.
  const missing = team && s.runner !== 'external' ? missingFor(team, health) : [];
  const loadError = s.tasks.error ?? s.team.error;
  return (
    <PageBody className="mx-auto w-full max-w-[1400px]">
      {loadError && <ErrorState error={loadError} />}
      <div className="grid gap-3 lg:grid-cols-[minmax(0,5fr)_minmax(0,7fr)]">
        <WorldCard benchmark={preview.bench?.name ?? health?.benchmark?.name ?? ''} twin={preview.twin} parts={partsOf(health)} tasks={s.tasks.tasks} task={s.task}
          onTask={s.setTaskId} scenario={s.scenario} busy={start.starting} />
        <TeamCard team={team} runner={s.runner} suggested={s.suggested} onImported={s.team.replace} />
      </div>
      <RunBar team={team} runner={s.runner} setRunner={s.setRunner} missing={missing} error={start.error} onRun={start.run}
        disabled={!team || !s.task || !!s.scenario.problem} />
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
    return <PageBody><LiveRun key={branchId} runId={runId} branchId={branchId} nameOf={s.nameOf} onAgain={start.run} onNew={() => setSp({})} /></PageBody>;
  }
  return <SetupView s={s} health={health} start={start} />;
}
