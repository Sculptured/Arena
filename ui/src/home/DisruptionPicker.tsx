import { X } from 'lucide-react';
import { useState } from 'react';
import { Button } from '../components/ui/button';
import { NativeSelect } from '../components/ui/input';
import { type ScenarioState, ParamInput } from '../scenario/ScenarioBuilder';
import { chosen, optionWords } from '../scenario/scenario';

/** "Add a disruption": the benchmark's disruptions as a picker; each one added shows in words, its parameters behind Adjust. */
export default function DisruptionPicker({ s, busy }: { s: ScenarioState; busy: boolean }) {
  const [adjusting, setAdjusting] = useState<string | null>(null);
  const options = s.options?.disruptions ?? [];
  if (!options.length) return null;
  const on = chosen(s.options, s.sel);
  const left = options.filter((d) => !on.includes(d));
  return (
    <div className="flex flex-col gap-1.5">
      <span className="text-2xs font-medium text-muted">Disruptions</span>
      {on.map((d) => {
        const words = optionWords(d, s.sel[d.id]?.params ?? {});
        return (
          <div key={d.id} className="flex flex-col gap-2 rounded-md border border-line bg-raised px-2.5 py-2">
            <div className="flex items-start gap-2">
              <span className="font-mono text-warn" aria-hidden="true">▼</span>
              <span className="flex min-w-0 flex-1 flex-col"><span className="text-sm font-medium">{words.label}</span><span className="text-xs text-muted">{words.description}</span></span>
              {d.params.length > 0 && <Button size="sm" variant="ghost" onClick={() => setAdjusting(adjusting === d.id ? null : d.id)} disabled={busy}>Adjust</Button>}
              <Button size="icon" variant="ghost" className="size-6" aria-label={`Remove ${words.label}`} onClick={() => s.toggle(d.id, false)} disabled={busy}><X /></Button>
            </div>
            {adjusting === d.id && (
              <div className="flex flex-wrap gap-2 pl-5">
                {d.params.map((p) => <ParamInput key={p.name} spec={p} value={s.sel[d.id].params[p.name]} onChange={(raw) => s.setParam(d.id, p, raw)} disabled={busy} />)}
              </div>
            )}
          </div>
        );
      })}
      {left.length > 0 && (
        <NativeSelect value="" onChange={(e) => { if (e.target.value) s.toggle(e.target.value, true); }} disabled={busy} aria-label="Add a disruption">
          <option value="">{on.length ? 'Add another disruption…' : 'Add a disruption…'}</option>
          {left.map((d) => <option key={d.id} value={d.id}>{d.label}</option>)}
        </NativeSelect>
      )}
      {s.problem && <p className="text-xs font-medium text-alarm" role="status">⚠ {s.problem}</p>}
    </div>
  );
}
