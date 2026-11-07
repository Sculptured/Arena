import { Field, Input, NativeSelect } from './components/ui/input';
import { Button } from './components/ui/button';
import {
  modelPlaceholder, modelProblem, providerOptions, providerState, stateGlyph, stateReason,
} from './providers';
import type { ProviderStatus } from './types';

/** The provider select of an API-mode agent; unavailable providers are disabled and say why. */
export function ProviderSelect({ value, list, onChange }: { value: string; list: ProviderStatus[]; onChange: (id: string) => void }) {
  return (
    <Field label="Provider" className="sm:w-56">
      <NativeSelect value={value} onChange={(e) => onChange(e.target.value)}>
        {providerOptions(list, value).map((o) => (
          <option key={o.id} value={o.id} disabled={o.disabled}>{o.text}</option>
        ))}
      </NativeSelect>
    </Field>
  );
}

/** A model box: the provider's listed models to pick from, or any id typed in. */
export function ModelInput({ listId, provider, value, list, placeholder, onChange }: {
  listId: string;
  /** The agent's provider; null for a CLI agent (no list, the CLI's default). */
  provider: string | null;
  value: string;
  list: ProviderStatus[];
  placeholder?: string;
  onChange: (model: string) => void;
}) {
  const models = (provider && list.find((s) => s.id === provider)?.models) || [];
  return (
    <Field label="Model" className="min-w-40 flex-1">
      <Input
        className="font-mono text-xs"
        list={models.length ? listId : undefined}
        value={value}
        placeholder={placeholder ?? (provider ? modelPlaceholder(provider) : 'default')}
        onChange={(e) => onChange(e.target.value)}
      />
      {models.length > 0 && (
        <datalist id={listId}>
          {models.map((m) => <option key={m} value={m} />)}
        </datalist>
      )}
    </Field>
  );
}

/** Under the provider and model: why the agent can't run, or where its models come from. */
export function ModelHint({ provider, model, list }: { provider: string; model: string | null | undefined; list: ProviderStatus[] }) {
  const problem = modelProblem(provider, model, list);
  if (problem) return <span className="text-xs font-medium text-alarm" role="status">⚠ {problem}</span>;
  const s = list.find((x) => x.id === provider);
  if (!s) return null;
  const hint = s.models?.length ? `${s.models.length} models listed by ${s.label}; or type any model id.`
    : providerState(s) === 'not checked' ? `${s.label} not checked yet; type a model id.`
      : s.why ? `Models not listed (${s.why}); type a model id.` : null;
  return hint ? <span className="text-xs text-muted">{hint}</span> : null;
}

/** One entry per provider: glyph, name, state word and reason; a button asks local servers. */
export function ProviderStatusLine({ statuses, loading, error, onCheck }: {
  statuses: ProviderStatus[];
  loading: boolean;
  error?: string | null;
  onCheck: () => void;
}) {
  const unchecked = statuses.some((s) => providerState(s) === 'not checked');
  return (
    <div className="flex flex-col gap-1.5">
      <ul className="flex flex-col gap-1">
        {statuses.map((s) => {
          const st = providerState(s);
          const why = stateReason(s);
          return (
            <li key={s.id} className="flex flex-wrap items-baseline gap-x-1 text-xs">
              <span aria-hidden="true" className="w-3 text-center font-mono text-muted">{stateGlyph(st)}</span> <span className="font-medium">{s.label}</span> <span className="text-muted">{st}</span>
              {why && <span className="text-muted">: <span className="font-mono text-2xs">{why}</span></span>}
            </li>
          );
        })}
      </ul>
      {statuses.length > 0 && (
        <Button size="sm" className="self-start" onClick={onCheck} disabled={loading}
          title="Ask each local server, and OpenAI and Anthropic when keyed, for their models (GET /api/providers)">
          {loading ? 'Checking…' : unchecked ? 'Check' : 'Check again'}
        </Button>
      )}
      {error && <span className="text-xs text-alarm">providers: {error}</span>}
    </div>
  );
}
