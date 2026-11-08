import { ArrowUp, Plus, RotateCcw, Trash2 } from 'lucide-react';
import { useEffect, useState } from 'react';
import { defaultTeam } from '../api';
import { ModelHint, ModelInput, ProviderSelect } from '../ProviderFields';
import { Button } from '../components/ui/button';
import { Checkbox, Field, Input, NativeSelect, Textarea } from '../components/ui/input';
import { PageBody, PageHeader } from '../components/ui/page';
import { Panel } from '../components/ui/panel';
import { ErrorState, LoadingState } from '../components/ui/state';
import { MODE_LABEL, providerOf, useProviderList } from '../providers';
import { clearStoredTeam, loadTeam, saveTeam, slug, withSlugIds } from '../team';
import type { AgentKind, AgentSpec, ProviderStatus, Team as TeamT } from '../types';

const KINDS: AgentKind[] = ['claude_code', 'codex', 'open'];

/** Mode, then (API mode) provider and model; a CLI agent's model is the CLI's. */
export function AgentFields({ agent: a, index: i, openOk, list, patch }: {
  agent: AgentSpec;
  index: number;
  /** Some provider can take API-mode calls. */
  openOk: boolean;
  list: ProviderStatus[];
  patch: (p: Partial<AgentSpec>) => void;
}) {
  const provider = a.kind === 'open' ? providerOf(a) : null;
  return (
    <>
      <div className="flex flex-wrap items-end gap-2">
        <Field label="Mode" className="w-full sm:w-52">
          <NativeSelect value={a.kind} onChange={(e) => patch({ kind: e.target.value as AgentKind, model: null, provider: undefined })}>
            {KINDS.map((k) => (
              <option key={k} value={k} disabled={k === 'open' && !openOk && a.kind !== 'open'}>
                {MODE_LABEL[k]}{k === 'open' && !openOk ? ' (no provider available)' : ''}
              </option>
            ))}
          </NativeSelect>
        </Field>
        {provider && <ProviderSelect value={provider} list={list} onChange={(id) => patch({ provider: id, model: null })} />}
        <ModelInput
          listId={`team-models-${i}`}
          provider={provider}
          value={a.model ?? ''}
          list={list}
          onChange={(m) => patch({ model: m.trim() === '' ? null : m })}
        />
      </div>
      {provider && <ModelHint provider={provider} model={a.model} list={list} />}
    </>
  );
}

/** The team as stored, saved on every edit once it has loaded. */
function useTeamEditor() {
  const [team, setTeam] = useState<TeamT | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  useEffect(() => {
    loadTeam().then((t) => { setTeam(t); setLoaded(true); }).catch((e) => setError(String(e.message ?? e)));
  }, []);
  useEffect(() => { if (loaded && team) saveTeam(team); }, [loaded, team]);
  const setAgents = (fn: (a: AgentSpec[]) => AgentSpec[]) => setTeam((t) => (t ? { ...t, agents: withSlugIds(fn(t.agents)) } : t));
  const ops = {
    patch: (i: number, p: Partial<AgentSpec>) => setAgents((as) => as.map((a, j) => (j === i ? { ...a, ...p } : a))),
    move: (i: number) => setAgents((as) => { const n = [...as]; [n[i - 1], n[i]] = [n[i], n[i - 1]]; return n; }),
    remove: (i: number) => setAgents((as) => as.filter((_, j) => j !== i)),
    add: () => setAgents((as) => [...as, { id: slug('agent', new Set(as.map((a) => a.id))), name: `Agent ${as.length + 1}`, kind: 'claude_code', model: null, prompt: '' }]),
    reset: async () => {
      clearStoredTeam();
      try { setTeam(await defaultTeam()); } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    },
  };
  return { team, error, ...ops };
}

function AgentCard({ a, i, count, openOk, list, ed }: {
  a: AgentSpec; i: number; count: number; openOk: boolean; list: ProviderStatus[]; ed: ReturnType<typeof useTeamEditor>;
}) {
  return (
    <Panel className="flex flex-col gap-3 p-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="grid size-6 shrink-0 place-items-center rounded-sm border border-line bg-raised font-mono text-xs text-muted">{i + 1}</span>
        <Input className="h-8 min-w-40 flex-1 text-base font-medium" value={a.name} onChange={(e) => ed.patch(i, { name: e.target.value })} placeholder="Name" aria-label="Agent name" title={`Agent id: ${a.id}`} />
        <label className="flex items-center gap-1.5 text-xs" title="Give this agent only the world's read tools">
          <Checkbox checked={!!a.read_only} onChange={(e) => ed.patch(i, { read_only: e.target.checked })} />Read-only
        </label>
        <Button size="icon" variant="ghost" onClick={() => ed.move(i)} disabled={i === 0} aria-label="Move up" title="Move up"><ArrowUp /></Button>
        <Button size="icon" variant="ghost" onClick={() => ed.remove(i)} disabled={count === 1} aria-label="Remove" title="Remove"><Trash2 /></Button>
      </div>
      <AgentFields agent={a} index={i} openOk={openOk} list={list} patch={(p) => ed.patch(i, p)} />
      <Field label="Prompt"><Textarea value={a.prompt} rows={5} onChange={(e) => ed.patch(i, { prompt: e.target.value })} /></Field>
    </Panel>
  );
}

export default function Team() {
  const { list } = useProviderList();
  const ed = useTeamEditor();
  // Any provider not known to be unavailable (a local server not checked yet counts).
  const openOk = list.some((s) => s.available || (s.kind === 'local' && !s.checked));
  return (
    <>
      <PageHeader title={<span title="Agents run in order, each handing off to the next; any agent may write unless you make it read-only. New runs and suites use this team. Edits are saved in this browser.">Team</span>}
        actions={<><Button variant="ghost" className="text-muted" onClick={ed.reset}><RotateCcw />Reset</Button><Button variant="primary" onClick={ed.add} disabled={!ed.team}><Plus />Add agent</Button></>} />
      <PageBody className="mx-auto w-full max-w-5xl">
        {ed.error && <ErrorState error={ed.error} />}
        {!ed.team && !ed.error && <Panel><LoadingState label="Loading your team…" /></Panel>}
        {ed.team?.agents.map((a, i) => <AgentCard key={i} a={a} i={i} count={ed.team!.agents.length} openOk={openOk} list={list} ed={ed} />)}
      </PageBody>
    </>
  );
}
