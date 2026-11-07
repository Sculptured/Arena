import { useEffect, useSyncExternalStore } from 'react';
import { providers as fetchProviders } from './api';
import { useHealth } from './components';
import { DEFAULT_OPENROUTER_MODEL, DEFAULT_PROVIDER } from './types';
import type { AgentKind, AgentSpec, ProviderKind, ProviderStatus, Team } from './types';

/** The cost source of a local server's calls (`providers/mod.rs` `LOCAL_COST_SOURCE`). */
export const LOCAL_COST_SOURCE = 'local (no API charge)';

export const MODE_LABEL: Record<AgentKind, string> = { claude_code: 'Claude Code', codex: 'Codex', open: 'API mode', external: 'External' };

/** The provider id `p` stands for: itself trimmed, else the default (`openrouter`). */
export function providerId(p: string | null | undefined): string {
  const t = p?.trim();
  return t ? t : DEFAULT_PROVIDER;
}

/** The provider an agent's model calls go to. */
export const providerOf = (a: { provider?: string | null }) => providerId(a.provider);

export function kindOf(id: string): ProviderKind {
  if (id === 'openrouter' || id === 'openai' || id === 'anthropic') return id;
  return 'local';
}

/** A short name: OpenRouter, OpenAI, Anthropic, or `<name> (local)`. */
export function providerName(id: string): string {
  switch (kindOf(id)) {
    case 'openrouter': return 'OpenRouter';
    case 'openai': return 'OpenAI';
    case 'anthropic': return 'Anthropic';
    default: return `${id.replace(/^local:/, '')} (local)`;
  }
}

/** Only OpenRouter has a default model; the others need one named. */
export const hasDefaultModel = (id: string) => kindOf(id) === 'openrouter';

/** OpenAI and Anthropic report no price, and arenad keeps no price table. */
export const costIsUnknown = (id: string) => kindOf(id) === 'openai' || kindOf(id) === 'anthropic';

/** The providers of a team's open agents. */
export const teamProviders = (team: Team | null | undefined): string[] =>
  [...new Set((team?.agents ?? []).filter((a) => a.kind === 'open').map(providerOf))];

/** "deepseek/… (default) · OpenRouter", "sonnet · Claude Code": the model and where it runs. */
export function agentModelLine(a: Pick<AgentSpec, 'kind' | 'model' | 'provider'>): string {
  const model = a.model?.trim();
  if (a.kind !== 'open') return model ? `${model} · ${MODE_LABEL[a.kind]}` : MODE_LABEL[a.kind];
  const p = providerOf(a);
  const m = model || (hasDefaultModel(p) ? `${DEFAULT_OPENROUTER_MODEL} (default)` : 'no model');
  return `${m} · ${providerName(p)}`;
}

export type ProviderState = 'ready' | 'unavailable' | 'not checked';

/** Ready, unavailable, or (a local server /api/health didn't ask) not checked. */
export function providerState(s: ProviderStatus): ProviderState {
  if (s.available) return 'ready';
  return s.kind === 'local' && !s.checked ? 'not checked' : 'unavailable';
}

const GLYPH: Record<ProviderState, string> = { ready: '✓', unavailable: '✕', 'not checked': '?' };
export const stateGlyph = (st: ProviderState) => GLYPH[st];

/** The reason shown next to the state (none for "not checked": the word says it). */
export const stateReason = (s: ProviderStatus): string | null =>
  providerState(s) === 'not checked' ? null : s.why ?? null;

/** "✕ OpenAI unavailable: OPENAI_API_KEY is not set". */
export function providerLine(s: ProviderStatus): string {
  const st = providerState(s);
  const why = stateReason(s);
  return `${GLYPH[st]} ${s.label} ${st}${why ? `: ${why}` : ''}`;
}

/** The status bar's one line about API mode: how many providers are ready, and the shared reason when none is. */
export function providerSummary(list: ProviderStatus[]): { glyph: string; text: string } | null {
  if (!list.length) return null;
  const ready = list.filter((s) => providerState(s) === 'ready').length;
  const unchecked = list.filter((s) => providerState(s) === 'not checked').length;
  const count = `${ready} of ${list.length} ready`;
  if (ready === list.length) return { glyph: GLYPH.ready, text: count };
  if (ready > 0) return { glyph: GLYPH.ready, text: unchecked ? `${count}, ${unchecked} not checked` : count };
  if (unchecked) return { glyph: GLYPH['not checked'], text: `${count}, ${unchecked} not checked` };
  return { glyph: GLYPH.unavailable, text: `${count}: ${sharedReason(list)}` };
}

/** "no API key set" when every provider lacks its key, the one reason they share, else a pointer to the list. */
function sharedReason(list: ProviderStatus[]): string {
  const why = list.map((s) => s.why ?? '');
  if (why.every((w) => /_API_KEY is not set$/.test(w))) return 'no API key set';
  return why.every((w) => w === why[0]) && why[0] ? why[0] : 'see each provider';
}

/** An option of a provider select: unavailable ones are disabled and say why. */
export interface ProviderOption {
  id: string;
  text: string;
  disabled: boolean;
}

/**
 * The providers to choose from. `current` (the agent's provider) is always listed and never
 * disabled, so a team keeps showing what it names; one the arena doesn't know is "not configured".
 */
export function providerOptions(list: ProviderStatus[], current: string): ProviderOption[] {
  const out: ProviderOption[] = list.map((s) => {
    const st = providerState(s);
    const why = stateReason(s);
    const text = st === 'ready' ? s.label : `${s.label} (${st}${why ? `: ${why}` : ''})`;
    return { id: s.id, text, disabled: st === 'unavailable' && s.id !== current };
  });
  if (!list.some((s) => s.id === current)) {
    out.push({ id: current, text: list.length ? `${current} (not configured)` : providerName(current), disabled: false });
  }
  return out;
}

/** Why an open agent on `provider` with `model` can't run, if the UI can tell. */
export function modelProblem(provider: string, model: string | null | undefined, list: ProviderStatus[]): string | null {
  const s = list.find((x) => x.id === provider);
  if (!s && list.length) return `${provider} is not configured on this arena`;
  if (s && providerState(s) === 'unavailable') return `${s.label} is unavailable${s.why ? `: ${s.why}` : ''}`;
  if (!model?.trim() && !hasDefaultModel(provider)) return `Name a model for ${s?.label ?? providerName(provider)}`;
  return null;
}

/** The model box's hint: the default, or that a model must be named. */
export const modelPlaceholder = (provider: string) =>
  hasDefaultModel(provider) ? `${DEFAULT_OPENROUTER_MODEL} (default)` : 'model id (required)';

// ---------------------------------------------------------------- the checked list, shared

/** The last `/api/providers` answer, shared by every page (and the status line). */
let checked: ProviderStatus[] | null = null;
let pending: Promise<ProviderStatus[]> | null = null;
let failure: string | null = null;
const listeners = new Set<() => void>();
const notify = () => listeners.forEach((f) => f());

interface Snapshot { list: ProviderStatus[] | null; loading: boolean; error: string | null }
let snapshot: Snapshot = { list: null, loading: false, error: null };
const refreshSnapshot = () => { snapshot = { list: checked, loading: pending != null, error: failure }; notify(); };

/** Asks `/api/providers` (once at a time) and shares the answer. */
export function checkProviders(): Promise<ProviderStatus[]> {
  if (pending) return pending;
  pending = fetchProviders()
    .then((v) => { checked = v; failure = null; return v; })
    .catch((e) => { failure = e instanceof Error ? e.message : String(e); throw e; })
    .finally(() => { pending = null; refreshSnapshot(); });
  refreshSnapshot();
  return pending;
}

/** For tests: forget the shared answer. */
export function resetProviders(list: ProviderStatus[] | null = null): void {
  checked = list; pending = null; failure = null; refreshSnapshot();
}

function subscribe(f: () => void) {
  listeners.add(f);
  return () => { listeners.delete(f); };
}

/** The checked provider list (`check`: ask for it on mount unless it's already known). */
export function useCheckedProviders(check = false): Snapshot {
  const s = useSyncExternalStore(subscribe, () => snapshot, () => snapshot);
  useEffect(() => {
    if (check && !checked) checkProviders().catch(() => {});
  }, [check]);
  return s;
}

/** The checked list once there is one, else the health summary (locals not checked). */
export const bestProviders = (checkedList: ProviderStatus[] | null, health: ProviderStatus[] | undefined) =>
  checkedList ?? health ?? [];

/** The providers to pick from: asks `/api/providers` on mount, the health summary until it answers. */
export function useProviderList(): { list: ProviderStatus[]; loading: boolean; error: string | null } {
  const health = useHealth();
  const s = useCheckedProviders(true);
  return { list: bestProviders(s.list, health?.providers), loading: s.loading, error: s.error };
}
