import { useEffect, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { tasks as fetchTasks } from '../api';
import { Dialog, DialogActions, DialogContent } from '../components/ui/dialog';
import { Checkbox, Field, Input, NativeSelect } from '../components/ui/input';
import { ErrorState } from '../components/ui/state';
import { createSuite } from '../control/api';
import ScenarioBuilder, { useScenario } from '../scenario/ScenarioBuilder';
import { loadTeam, teamForRun } from '../team';
import type { Runner, TaskSummary } from '../types';

export const RUNNERS: Record<Runner, string> = { chain: 'one after another', dispatch: 'a lead dispatching', concurrent: 'all at once', external: 'external system' };

function Scenarios({ tasks, picked, toggle, busy }: { tasks: TaskSummary[] | null; picked: string[]; toggle: (id: string) => void; busy: boolean }) {
  return (
    <fieldset className="flex flex-col gap-1">
      <legend className="mb-1 text-2xs font-medium text-muted">Scenarios {picked.length ? `(${picked.length} picked)` : '(all, unless you pick some)'}</legend>
      {!tasks && <span className="text-xs text-muted">Loading scenarios…</span>}
      <div className="max-h-56 overflow-auto rounded-md border border-line bg-raised">
        {tasks?.map((t) => (
          <label key={t.id} className="flex items-start gap-2 border-b border-line px-2.5 py-1.5 text-xs last:border-b-0 hover:bg-hover">
            <Checkbox className="mt-0.5" checked={picked.includes(t.id)} onChange={() => toggle(t.id)} disabled={busy} />
            <span className="font-mono">{t.id}</span><span className="text-muted">{t.reason}</span>
          </label>
        ))}
      </div>
    </fieldset>
  );
}

function useNewSuite(tasks: TaskSummary[] | null, open: boolean) {
  const nav = useNavigate();
  const [picked, setPicked] = useState<string[]>([]);
  const all = (tasks ?? []).map((t) => t.id);
  const scenario = useScenario(open, picked.length ? picked : all);
  const [repeats, setRepeats] = useState(2);
  const [runner, setRunner] = useState<Runner>('chain');
  // Your team's own runner (a suggested team's), each time the dialog opens.
  useEffect(() => { if (open) loadTeam().then((t) => setRunner(t.runner ?? 'chain')).catch(() => {}); }, [open]);
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const toggle = (id: string) => setPicked((p) => (p.includes(id) ? p.filter((x) => x !== id) : [...p, id]));
  const start = async () => {
    setBusy(true); setError(null);
    try {
      const chosen = all.filter((t) => picked.includes(t));
      const r = await createSuite({ name: name.trim() || undefined, team: teamForRun(await loadTeam(), runner), tasks: chosen, repeats, runner, scenario: scenario.scenario });
      nav(`/suites/${r.id}`);
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    setBusy(false);
  };
  return { picked, toggle, repeats, setRepeats, runner, setRunner, name, setName, busy, error, start, scenario };
}

/** Run your team over each scenario, repeated, on the same seeded world. */
export default function NewSuiteDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  const [tasks, setTasks] = useState<TaskSummary[] | null>(null);
  useEffect(() => { if (open) fetchTasks().then(setTasks).catch(() => setTasks([])); }, [open]);
  const f = useNewSuite(tasks, open);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title="New suite" description={<>Runs your <Link to="/team" className="text-accent hover:underline">team</Link> over each scenario on the same seeded world, so suites compare fairly.</>}>
        <div className="flex flex-col gap-3 overflow-auto p-4">
          <Field label="Name"><Input value={f.name} onChange={(e) => f.setName(e.target.value)} placeholder="your team's name" disabled={f.busy} /></Field>
          <div className="grid grid-cols-2 gap-3">
            <Field label="Repeats"><Input type="number" min={1} max={100} value={f.repeats} onChange={(e) => f.setRepeats(Number(e.target.value))} disabled={f.busy} /></Field>
            <Field label="Runs made by">
              <NativeSelect value={f.runner} onChange={(e) => f.setRunner(e.target.value as Runner)} disabled={f.busy}>
                {Object.entries(RUNNERS).map(([k, v]) => <option key={k} value={k}>{v}</option>)}
              </NativeSelect>
            </Field>
          </div>
          <Scenarios tasks={tasks} picked={f.picked} toggle={f.toggle} busy={f.busy} />
          <ScenarioBuilder s={f.scenario} busy={f.busy} />
          {f.error && <ErrorState error={f.error} hint="Nothing was started. Fix the cause and start again." />}
        </div>
        <DialogActions onCancel={() => onOpenChange(false)} onGo={f.start} disabled={f.busy || !tasks?.length || !!f.scenario.problem} label={f.busy ? 'Starting…' : 'Start suite'} />
      </DialogContent>
    </Dialog>
  );
}
