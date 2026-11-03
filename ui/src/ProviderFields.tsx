import {
  modelPlaceholder, modelProblem, providerOptions, providerState, stateGlyph, stateReason,
} from './providers';
import type { ProviderStatus } from './types';

/** The provider select of an API-mode agent; unavailable providers are disabled and say why. */
export function ProviderSelect({ value, list, onChange }: { value: string; list: ProviderStatus[]; onChange: (id: string) => void }) {
  return (
    <label className="field provider-field">
      <span className="label">Provider</span>
      <select value={value} onChange={(e) => onChange(e.target.value)}>
        {providerOptions(list, value).map((o) => (
          <option key={o.id} value={o.id} disabled={o.disabled}>{o.text}</option>
        ))}
      </select>
    </label>
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
    <label className="field grow">
      <span className="label">Model</span>
      <input
        className="mono"
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
    </label>
  );
}

/** Under the provider and model: why the agent can't run, or where its models come from. */
export function ModelHint({ provider, model, list }: { provider: string; model: string | null | undefined; list: ProviderStatus[] }) {
  const problem = modelProblem(provider, model, list);
  if (problem) return <span className="small field-problem" role="status">{problem}</span>;
  const s = list.find((x) => x.id === provider);
  if (!s) return null;
  if (s.models?.length) return <span className="muted small">{s.models.length} models listed by {s.label}; or type any model id.</span>;
  if (providerState(s) === 'not checked') return <span className="muted small">{s.label} not checked yet; type a model id.</span>;
  if (s.why) return <span className="muted small">Models not listed ({s.why}); type a model id.</span>;
  return null;
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
    <span className="provider-line">
      {statuses.map((s) => {
        const st = providerState(s);
        const why = stateReason(s);
        return (
          <span key={s.id} className={`prov prov-${st.replace(' ', '-')}`}>
            <span className="prov-glyph" aria-hidden="true">{stateGlyph(st)}</span> {s.label} <span className="prov-word">{st}</span>
            {why && <span className="prov-why">: {why}</span>}
          </span>
        );
      })}
      {statuses.length > 0 && (
        <button className="btn btn-sm" onClick={onCheck} disabled={loading} title="Ask each local server, and OpenAI and Anthropic when keyed, for their models (GET /api/providers)">
          {loading ? 'Checking…' : unchecked ? 'Check' : 'Check again'}
        </button>
      )}
      {error && <span className="prov-why">providers: {error}</span>}
    </span>
  );
}
