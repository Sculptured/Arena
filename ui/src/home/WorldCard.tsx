import { NativeSelect } from '../components/ui/input';
import type { ScenarioState } from '../scenario/ScenarioBuilder';
import { chosen, optionWords } from '../scenario/scenario';
import { taskName, ticketTitle, worldName } from './summary';
import type { Task } from './useHome';

/** The task by its plain name; the ticket heading is its tooltip. */
export function TaskPicker({ tasks, task, onTask, busy }: { tasks: Task[] | null; task: Task | null; onTask: (id: string) => void; busy: boolean }) {
  const tip = task ? [ticketTitle(task.ticket), task.reason].filter(Boolean).join('\n') : undefined;
  return (
    <NativeSelect value={task?.id ?? ''} onChange={(e) => onTask(e.target.value)} disabled={busy || !tasks?.length} title={tip} aria-label="Task" className="h-10 text-base">
      {!tasks && <option>Loading tasks…</option>}
      {tasks?.map((t) => <option key={t.id} value={t.id}>{taskName(t)}</option>)}
    </NativeSelect>
  );
}

/** The disruptions chosen under Options, in words, so a run never starts with a hidden one. */
function Chosen({ s }: { s: ScenarioState }) {
  const on = chosen(s.options, s.sel);
  if (!on.length) return null;
  return (
    <ul className="flex flex-col gap-0.5 text-sm">
      {on.map((d) => {
        const w = optionWords(d, s.sel[d.id]?.params ?? {});
        return <li key={d.id} title={w.description}><span className="font-mono text-warn" aria-hidden="true">▼ </span>{w.label}</li>;
      })}
    </ul>
  );
}

/** Before a run: the world's name, the task picker, and any disruption chosen. */
export default function WorldCard({ benchmark, title, tasks, task, onTask, scenario, busy }: {
  benchmark: string; title?: string; tasks: Task[] | null; task: Task | null;
  onTask: (id: string) => void; scenario: ScenarioState; busy: boolean;
}) {
  return (
    <div className="flex flex-col gap-3">
      <h1 className="text-xl font-semibold">{worldName(benchmark, title)}</h1>
      <TaskPicker tasks={tasks} task={task} onTask={onTask} busy={busy} />
      <Chosen s={scenario} />
    </div>
  );
}
