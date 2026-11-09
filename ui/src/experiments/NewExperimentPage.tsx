import { Play, Plus } from 'lucide-react';
import { type ReactNode, useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import ImportDialog from '../board/ImportDialog';
import { useHealth } from '../components';
import { Button } from '../components/ui/button';
import { PageBody } from '../components/ui/page';
import { Segmented } from '../components/ui/segmented';
import { ErrorState } from '../components/ui/state';
import DisruptionPicker from '../home/DisruptionPicker';
import { missingFor, worldName } from '../home/summary';
import { TaskPicker } from '../home/WorldCard';
import { useSuggestedTeams, useTasks, useTeam, useWorldPreview } from '../home/useHome';
import { useScenario } from '../scenario/ScenarioBuilder';
import { teamForRun } from '../team';
import type { Health, Team } from '../types';
import { createExperiment } from './api';
import { DEFAULT_RUNS, MAX_SETUPS, RUN_CHOICES, type SetupDraft, newDraft, setupTeam, setupsProblem } from './model';
import SetupCard, { type TeamChoices } from './SetupCard';
import type { NewExperiment } from './types';

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-3">
      <h2 className="text-xs font-medium text-muted">{title}</h2>
      {children}
    </section>
  );
}

/** The benchmark's teams, then yours when it isn't one of them, then the ones imported here. */
function useTeamChoices() {
  const suggested = useSuggestedTeams();
  const yours = useTeam().team;
  const [imported, setImported] = useState<Team[]>([]);
  const choices: TeamChoices = useMemo(() => [
    ...suggested.map((t) => ({ key: `s:${t.id}`, label: t.name, team: t })),
    ...(yours && !suggested.some((t) => t.id === yours.id) ? [{ key: 'yours', label: `${yours.name} (yours)`, team: yours }] : []),
    ...imported.map((t, i) => ({ key: `i:${i}`, label: `${t.name} (imported)`, team: t })),
  ], [suggested, yours, imported]);
  const add = (t: Team) => { setImported((l) => [...l, t]); return `i:${imported.length}`; };
  return { choices, add };
}

/** The setups being written, starting with one on the first team offered; each new one copies the last. */
function useDrafts(firstTeam: string | undefined) {
  const [drafts, setDrafts] = useState<SetupDraft[]>([]);
  useEffect(() => { if (firstTeam && !drafts.length) setDrafts([newDraft(0, firstTeam)]); }, [firstTeam, drafts.length]);
  return {
    drafts,
    set: (i: number, d: SetupDraft) => setDrafts((l) => l.map((x, j) => (j === i ? d : x))),
    add: () => setDrafts((l) => [...l, newDraft(l.length, l[l.length - 1].teamId, l[l.length - 1])]),
    remove: (i: number) => setDrafts((l) => l.filter((_, j) => j !== i)),
  };
}

/** Everything the page holds, what it comes to, and why it can't start yet. */
function useNewExperiment(health: Health | null) {
  const { tasks, error: tasksError } = useTasks();
  const teams = useTeamChoices();
  const drafts = useDrafts(teams.choices[0]?.key);
  const [taskId, setTaskId] = useState('');
  const [name, setName] = useState('');
  const [runs, setRuns] = useState<number>(DEFAULT_RUNS);
  const task = tasks?.find((t) => t.id === taskId) ?? tasks?.[0] ?? null;
  const scenario = useScenario(true, task ? [task.id] : []);
  const built = drafts.drafts.map((x) => { const base = teams.choices.find((c) => c.key === x.teamId)?.team; return base ? setupTeam(base, x) : null; });
  const missing = built.flatMap((t) => (t ? missingFor(t, health) : []));
  const problem = setupsProblem(drafts.drafts) ?? scenario.problem ?? missing[0] ?? null;
  const body = (): NewExperiment | null => (task && built.every(Boolean) ? {
    name: name.trim() || undefined, task: task.id, scenario: scenario.scenario, runs,
    setups: drafts.drafts.map((x, i) => { const t = built[i]!; const runner = t.runner ?? 'chain'; return { name: x.name.trim(), team: teamForRun(t, runner), runner }; }),
  } : null);
  return { tasks, tasksError, teams, drafts, task, setTaskId, name, setName, runs, setRuns, scenario, built, problem, body };
}

type Form = ReturnType<typeof useNewExperiment>;

function useStart() {
  const nav = useNavigate();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const start = async (body: NewExperiment | null) => {
    if (!body) return;
    setBusy(true); setError(null);
    try { nav(`/experiments/${(await createExperiment(body)).id}`); } catch (e) { setError(e instanceof Error ? e.message : String(e)); setBusy(false); }
  };
  return { busy, error, start };
}

function Scenario({ f, world, busy }: { f: Form; world: string; busy: boolean }) {
  return (
    <Section title={`Scenario · ${world}`}>
      <TaskPicker tasks={f.tasks} task={f.task} onTask={f.setTaskId} busy={busy} />
      <DisruptionPicker s={f.scenario} busy={busy} />
    </Section>
  );
}

function Setups({ f, busy, onImport }: { f: Form; busy: boolean; onImport: (i: number) => void }) {
  const { drafts } = f.drafts;
  return (
    <Section title="Agent setups">
      <div className="grid gap-4 sm:grid-cols-2">
        {drafts.map((x, i) => (
          <SetupCard key={x.key} d={x} teams={f.teams.choices} team={f.built[i]} busy={busy} onChange={(n) => f.drafts.set(i, n)}
            onRemove={drafts.length > 1 ? () => f.drafts.remove(i) : undefined} onImport={() => onImport(i)} />
        ))}
        {drafts.length > 0 && drafts.length < MAX_SETUPS && (
          <button type="button" onClick={f.drafts.add} disabled={busy}
            className="flex min-h-40 flex-col items-center justify-center gap-1.5 rounded-lg border border-dashed border-line-strong text-sm text-muted transition-colors hover:border-accent hover:text-fg">
            <Plus className="size-4" />Add a setup to compare
          </button>
        )}
      </div>
    </Section>
  );
}

/** "2 setups × 5 runs = 10 runs", what blocks starting, and the one primary button. */
function RunBar({ f, s }: { f: Form; s: ReturnType<typeof useStart> }) {
  const n = f.drafts.drafts.length;
  const why = s.error ?? f.problem;
  return (
    <div className="sticky bottom-0 -mx-4 flex flex-wrap items-center gap-3 border-t border-line bg-bg/90 px-4 py-3 backdrop-blur sm:-mx-6 sm:px-6">
      <span className="text-sm text-muted">{n} setup{n === 1 ? '' : 's'} × {f.runs} run{f.runs === 1 ? '' : 's'} = <span className="font-medium text-fg">{n * f.runs} runs</span></span>
      {why && <span role="status" className="min-w-0 text-sm font-medium text-alarm">⚠ {why}</span>}
      <Button variant="primary" className="ml-auto h-9 px-5 text-sm" onClick={() => s.start(f.body())} disabled={!f.body() || !!f.problem || s.busy}>
        <Play />{s.busy ? 'Starting…' : 'Run experiment'}
      </Button>
    </div>
  );
}

/** Pick the scenario, write 1–4 agent setups, choose how many runs each, and run them all. */
export default function NewExperimentPage() {
  const health = useHealth();
  const preview = useWorldPreview();
  const f = useNewExperiment(health);
  const s = useStart();
  const [importFor, setImportFor] = useState<number | null>(null);
  const world = worldName(preview.bench?.name ?? health?.benchmark?.name ?? '', preview.twin?.title);
  return (
    <PageBody width="default" className="gap-8 pt-8 pb-0">
      <input value={f.name} onChange={(e) => f.setName(e.target.value)} placeholder="New experiment" aria-label="Experiment name"
        className="w-full rounded-sm bg-transparent text-xl font-semibold tracking-tight outline-none placeholder:text-fg focus:placeholder:text-faint focus-visible:ring-2 focus-visible:ring-accent" />
      {f.tasksError && <ErrorState error={f.tasksError} />}
      <Scenario f={f} world={world} busy={s.busy} />
      <Setups f={f} busy={s.busy} onImport={setImportFor} />
      <Section title="Runs per setup">
        <Segmented label="Runs per setup" value={f.runs} onChange={f.setRuns} disabled={s.busy} options={RUN_CHOICES.map((n) => ({ id: n, label: String(n) }))} />
      </Section>
      <RunBar f={f} s={s} />
      <ImportDialog open={importFor != null} onOpenChange={(o) => { if (!o) setImportFor(null); }}
        onImported={(t) => { if (importFor != null) f.drafts.set(importFor, { ...f.drafts.drafts[importFor], teamId: f.teams.add(t) }); setImportFor(null); }} />
    </PageBody>
  );
}
