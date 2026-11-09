import { Play, Plus } from 'lucide-react';
import { type ReactNode, useEffect, useMemo, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import ImportDialog from '../board/ImportDialog';
import { type ConnectFrom, type ConnectReturn, connectedLabel, connectedTeams } from '../connect/connect';
import { useHealth } from '../components';
import { Button } from '../components/ui/button';
import { PageBody } from '../components/ui/page';
import { Segmented } from '../components/ui/segmented';
import { ErrorState } from '../components/ui/state';
import DisruptionPicker from '../home/DisruptionPicker';
import { missingFor, runnerProblem, worldName } from '../home/summary';
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

/** The page's own state, carried through Connect your agent and back. */
interface Kept { name: string; taskId: string; runs: number; drafts: SetupDraft[] }

/** What Connect gave back when it sent the user here (read once, on arrival). */
function useReturned(): ConnectReturn & { kept: Kept | null } {
  const loc = useLocation();
  const nav = useNavigate();
  const [r] = useState(() => {
    const s = (loc.state ?? {}) as ConnectReturn;
    const kept = s.form && typeof s.form === 'object' && Array.isArray((s.form as Kept).drafts) ? (s.form as Kept) : null;
    return { ...s, kept };
  });
  // A reload starts fresh rather than applying the same return again.
  useEffect(() => { if (loc.state) nav(loc.pathname, { replace: true, state: null }); }, [loc.state, loc.pathname, nav]);
  return r;
}

/** The benchmark's teams, then yours when it isn't one of them, then your connected agents, then the ones imported here. */
function useTeamChoices(returned: Team | undefined) {
  const suggested = useSuggestedTeams();
  const yours = useTeam().team;
  const [imported, setImported] = useState<Team[]>([]);
  // The kept list, plus the agent just connected when this browser couldn't keep it.
  const [connected] = useState(() => { const l = connectedTeams(); return returned && !l.some((t) => t.id === returned.id) ? [returned, ...l] : l; });
  const choices: TeamChoices = useMemo(() => [
    ...suggested.map((t) => ({ key: `s:${t.id}`, label: t.name, team: t })),
    ...(yours && !suggested.some((t) => t.id === yours.id) ? [{ key: 'yours', label: `${yours.name} (yours)`, team: yours }] : []),
    ...connected.map((t) => ({ key: `c:${t.id}`, label: connectedLabel(t, connected), team: t })),
    ...imported.map((t, i) => ({ key: `i:${i}`, label: `${t.name} (imported)`, team: t })),
  ], [suggested, yours, connected, imported]);
  const add = (t: Team) => { setImported((l) => [...l, t]); return `i:${imported.length}`; };
  return { choices, add };
}

/** The setups being written, starting with one on the first team offered; each new one copies the last. */
function useDrafts(firstTeam: string | undefined, initial: SetupDraft[]) {
  const [drafts, setDrafts] = useState<SetupDraft[]>(initial);
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
  const back = useReturned();
  const teams = useTeamChoices(back.connected);
  // A new setup starts on the benchmark's team (or yours), not on an agent connected earlier.
  const drafts = useDrafts(teams.choices.find((c) => !/^[ci]:/.test(c.key))?.key, initialDrafts(back));
  const kept = back.kept ?? { name: '', taskId: '', runs: DEFAULT_RUNS };
  const [taskId, setTaskId] = useState(kept.taskId);
  const [name, setName] = useState(kept.name);
  const [runs, setRuns] = useState<number>(kept.runs);
  const task = tasks?.find((t) => t.id === taskId) ?? tasks?.[0] ?? null;
  const scenario = useScenario(true, task ? [task.id] : []);
  const built = drafts.drafts.map((x) => { const base = teams.choices.find((c) => c.key === x.teamId)?.team; return base ? setupTeam(base, x) : null; });
  const missing = built.flatMap((t) => (t ? setupMissing(t, health) : []));
  const problem = setupsProblem(drafts.drafts) ?? scenario.problem ?? missing[0] ?? null;
  const body = (): NewExperiment | null => (task && built.every(Boolean) ? {
    name: name.trim() || undefined, task: task.id, scenario: scenario.scenario, runs,
    setups: drafts.drafts.map((x, i) => { const t = built[i]!; const runner = t.runner ?? 'chain'; return { name: x.name.trim(), team: teamForRun(t, runner), runner }; }),
  } : null);
  const keep = (): Kept => ({ name, taskId, runs, drafts: drafts.drafts });
  return { tasks, tasksError, teams, drafts, task, setTaskId, name, setName, runs, setRuns, scenario, built, problem, body, keep };
}

/** What keeps a setup's team from running: a CLI or provider missing, or an HTTP agent under a dispatching lead. */
function setupMissing(t: Team, health: Health | null): string[] {
  const runner = runnerProblem(t, t.runner ?? 'chain');
  return runner ? [...missingFor(t, health), runner] : missingFor(t, health);
}

/** The setups on arrival: as they were before Connect, the connected agent's team in the setup that asked for it. */
function initialDrafts(back: ReturnType<typeof useReturned>): SetupDraft[] {
  const kept = back.kept?.drafts ?? [];
  const team = back.connected;
  if (!team) return kept;
  const key = `c:${team.id}`;
  if (!kept.length) return [{ ...newDraft(0, key), name: team.name }];
  return kept.map((d, i) => (i === (back.setup ?? 0) ? { ...d, teamId: key } : d));
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

function Setups({ f, busy, onImport, onConnect }: { f: Form; busy: boolean; onImport: (i: number) => void; onConnect: (i: number) => void }) {
  const { drafts } = f.drafts;
  return (
    <Section title="Agent setups">
      <div className="grid gap-4 sm:grid-cols-2">
        {drafts.map((x, i) => (
          <SetupCard key={x.key} d={x} teams={f.teams.choices} team={f.built[i]} busy={busy} onChange={(n) => f.drafts.set(i, n)}
            onRemove={drafts.length > 1 ? () => f.drafts.remove(i) : undefined} onImport={() => onImport(i)} onConnect={() => onConnect(i)} />
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
  const nav = useNavigate();
  const connect = (i: number) => { const from: ConnectFrom = { from: '/experiments/new', setup: i, form: f.keep() }; nav('/connect', { state: from }); };
  const world = worldName(preview.bench?.name ?? health?.benchmark?.name ?? '', preview.twin?.title);
  return (
    <PageBody width="default" className="gap-8 pt-8 pb-0">
      <input value={f.name} onChange={(e) => f.setName(e.target.value)} placeholder="New experiment" aria-label="Experiment name"
        className="w-full rounded-sm bg-transparent text-xl font-semibold tracking-tight outline-none placeholder:text-fg focus:placeholder:text-faint focus-visible:ring-2 focus-visible:ring-accent" />
      {f.tasksError && <ErrorState error={f.tasksError} />}
      <Scenario f={f} world={world} busy={s.busy} />
      <Setups f={f} busy={s.busy} onImport={setImportFor} onConnect={connect} />
      <Section title="Runs per setup">
        <Segmented label="Runs per setup" value={f.runs} onChange={f.setRuns} disabled={s.busy} options={RUN_CHOICES.map((n) => ({ id: n, label: String(n) }))} />
      </Section>
      <RunBar f={f} s={s} />
      <ImportDialog open={importFor != null} onOpenChange={(o) => { if (!o) setImportFor(null); }}
        onImported={(t) => { if (importFor != null) f.drafts.set(importFor, { ...f.drafts.drafts[importFor], teamId: f.teams.add(t) }); setImportFor(null); }} />
    </PageBody>
  );
}
