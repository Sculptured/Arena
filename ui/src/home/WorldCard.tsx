import { Field, NativeSelect } from '../components/ui/input';
import { Panel, PanelHeader } from '../components/ui/panel';
import type { ScenarioState } from '../scenario/ScenarioBuilder';
import TwinCanvas from '../world/TwinCanvas';
import { diffViews } from '../world/twin';
import type { TwinView } from '../world/types';
import { useWidth } from '../world/useWorld';
import DisruptionPicker from './DisruptionPicker';
import { partWord, taskName, ticketTitle, worldName } from './summary';
import type { Task } from './useHome';

/** The width the twin is drawn at before it is shrunk into the card. */
const TWIN_W = 1180;
const THUMB_H = 150;

/** The digital twin, shrunk to a thumbnail; the world's parts when it draws none yet. */
function Thumbnail({ twin, parts }: { twin: TwinView | null; parts: string[] }) {
  const { ref, width } = useWidth<HTMLDivElement>();
  if (!twin) {
    return (
      <div className="flex flex-wrap gap-1.5 rounded-md border border-line bg-sunken p-2.5">
        {parts.map((p) => <span key={p} className="rounded-sm border border-equip-line bg-equip px-2 py-1 text-xs font-medium">{partWord(p)}</span>)}
      </div>
    );
  }
  const scale = width ? width / TWIN_W : 0;
  return (
    <div ref={ref} className="pointer-events-none relative overflow-hidden rounded-md border border-line bg-sunken" style={{ height: THUMB_H }} aria-hidden="true">
      {scale > 0 && (
        <div className="absolute top-1 left-0 origin-top-left" style={{ width: TWIN_W, transform: `scale(${scale})` }}>
          <TwinCanvas view={twin} changes={diffViews(null, twin)} />
        </div>
      )}
    </div>
  );
}

function TaskPicker({ tasks, task, onTask, busy }: { tasks: Task[] | null; task: Task | null; onTask: (id: string) => void; busy: boolean }) {
  return (
    <Field label="Task">
      <NativeSelect value={task?.id ?? ''} onChange={(e) => onTask(e.target.value)} disabled={busy || !tasks?.length} title={task?.reason}>
        {!tasks && <option>Loading tasks…</option>}
        {tasks?.map((t) => <option key={t.id} value={t.id}>{taskName(t)}</option>)}
      </NativeSelect>
    </Field>
  );
}

/** Before a run: the world and its task in plain words, its twin, the task picker and the disruptions. */
export default function WorldCard({ benchmark, twin, parts, tasks, task, onTask, scenario, busy }: {
  benchmark: string; twin: TwinView | null; parts: string[]; tasks: Task[] | null; task: Task | null;
  onTask: (id: string) => void; scenario: ScenarioState; busy: boolean;
}) {
  const headline = task && (ticketTitle(task.ticket) || taskName(task));
  return (
    <Panel className="flex flex-col">
      <PanelHeader title="World" />
      <div className="flex flex-col gap-3 p-3">
        <div className="flex flex-col">
          <h2 className="text-base font-semibold">{worldName(benchmark, twin?.title)}</h2>
          <p className="text-sm text-muted">{headline || 'Loading the tasks…'}</p>
        </div>
        <Thumbnail twin={twin} parts={parts} />
        <TaskPicker tasks={tasks} task={task} onTask={onTask} busy={busy} />
        <DisruptionPicker s={scenario} busy={busy} />
      </div>
    </Panel>
  );
}
