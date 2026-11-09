import { X } from 'lucide-react';
import { Button } from '../components/ui/button';
import { Field, NativeSelect, Textarea } from '../components/ui/input';
import { runLine } from '../home/summary';
import type { Team } from '../types';
import { MODELS, type SetupDraft } from './model';

/** The teams a setup can pick, by key: the benchmark's, yours, and any imported here. */
export type TeamChoices = { key: string; label: string; team: Team }[];

/** Picking this option opens the import dialog instead of choosing a team. */
export const IMPORT = '__import__';

/** Picking this option opens Connect your agent instead of choosing a team. */
export const CONNECT = '__connect__';

/** One agent setup: its name, team, model and rules, and what that comes to in one line. */
export default function SetupCard({ d, teams, team, onChange, onRemove, onImport, onConnect, busy }: {
  d: SetupDraft; teams: TeamChoices; team: Team | null; onChange: (d: SetupDraft) => void; onRemove?: () => void; onImport: () => void; onConnect: () => void; busy: boolean;
}) {
  const set = (patch: Partial<SetupDraft>) => onChange({ ...d, ...patch });
  const claude = !!team?.agents.some((a) => a.kind === 'claude_code');
  return (
    <div className="flex min-w-0 flex-col gap-3 rounded-lg border border-line bg-surface p-4 shadow-panel">
      <div className="flex items-center gap-2">
        <input value={d.name} onChange={(e) => set({ name: e.target.value })} disabled={busy} aria-label="Setup name"
          className="min-w-0 flex-1 rounded-sm bg-transparent text-sm font-semibold outline-none placeholder:text-faint focus-visible:ring-2 focus-visible:ring-accent" />
        {onRemove && <Button size="icon" variant="ghost" className="size-6" aria-label={`Remove ${d.name}`} onClick={onRemove} disabled={busy}><X /></Button>}
      </div>
      <Field label="Team">
        <NativeSelect value={d.teamId} disabled={busy} onChange={(e) => (e.target.value === IMPORT ? onImport() : e.target.value === CONNECT ? onConnect() : set({ teamId: e.target.value }))}>
          {teams.map((t) => <option key={t.key} value={t.key}>{t.label}</option>)}
          <option value={IMPORT}>Import a team…</option>
          <option value={CONNECT}>Connect your agent…</option>
        </NativeSelect>
      </Field>
      {claude && (
        <Field label="Model">
          <NativeSelect value={d.model} onChange={(e) => set({ model: e.target.value })} disabled={busy}>
            {MODELS.map((m) => <option key={m.id} value={m.id}>{m.label}</option>)}
          </NativeSelect>
        </Field>
      )}
      <Field label="Rules">
        <Textarea rows={3} value={d.rules} onChange={(e) => set({ rules: e.target.value })} disabled={busy}
          placeholder="Optional. Added to every agent's prompt." className="resize-y font-sans text-sm" />
      </Field>
      <p className="mt-auto truncate text-xs text-muted" title={team ? runLine(team) : undefined}>{team ? runLine(team) : 'Loading the team…'}</p>
    </div>
  );
}
