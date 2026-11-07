import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ReactElement, ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { modesLine } from './components';
import { ProviderSelect, ProviderStatusLine } from './ProviderFields';
import {
  agentModelLine, bestProviders, checkProviders, modelProblem, providerLine, providerOptions, providerSummary, resetProviders,
} from './providers';
import { AgentFields } from './pages/Team';
import { AgentEdit, applied, forkProblem } from './run/ForkPanel';
import { changedFields, formatCost } from './run/timeline';
import { teamForRun } from './team';
import type { AgentSpec, ProviderStatus } from './types';

/** `/api/health`'s summary with only OpenRouter keyed and one local server (not asked). */
const HEALTH: ProviderStatus[] = [
  { id: 'openrouter', kind: 'openrouter', label: 'OpenRouter', available: true, checked: false },
  { id: 'openai', kind: 'openai', label: 'OpenAI', available: false, checked: false, why: 'OPENAI_API_KEY is not set' },
  { id: 'anthropic', kind: 'anthropic', label: 'Anthropic (Claude)', available: false, checked: false, why: 'ANTHROPIC_API_KEY is not set' },
  { id: 'local:ollama', kind: 'local', label: 'ollama (local, 127.0.0.1:11434)', available: false, checked: false,
    why: 'not checked (GET /api/providers asks the server)' },
];
/** `/api/providers`: the local server answered with its models. */
const CHECKED: ProviderStatus[] = HEALTH.map((s) =>
  s.kind === 'local' ? { ...s, available: true, checked: true, why: undefined, models: ['llama3.3', 'qwen3:8b'] } : s);

/** The text a reader sees in server-rendered markup. */
const text = (html: string) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;/g, "'").replace(/\s+/g, ' ').trim();

/** The first element in `node`'s tree (not rendered: components stay elements) that `pred` matches. */
type Props = { children?: ReactNode; onChange?: (v: string) => void };
function find(node: ReactNode, pred: (e: ReactElement<Props>) => boolean): ReactElement<Props> | null {
  if (Array.isArray(node)) {
    for (const n of node) {
      const f = find(n, pred);
      if (f) return f;
    }
    return null;
  }
  if (!node || typeof node !== 'object' || !('props' in node)) return null;
  const e = node as ReactElement<Props>;
  return pred(e) ? e : find(e.props.children, pred);
}

const agent = (over: Partial<AgentSpec> = {}): AgentSpec =>
  ({ id: 'resolver', name: 'Resolver', kind: 'open', model: null, prompt: 'p', ...over });

afterEach(() => { vi.unstubAllGlobals(); resetProviders(); });

describe('health line', () => {
  it('one entry per provider with glyph, word and reason; locals not checked', () => {
    const html = renderToStaticMarkup(<ProviderStatusLine statuses={HEALTH} loading={false} onCheck={() => {}} />);
    const t = text(html);
    expect(t).toContain('✓ OpenRouter ready');
    expect(t).toContain('✕ OpenAI unavailable : OPENAI_API_KEY is not set');
    expect(t).toContain('✕ Anthropic (Claude) unavailable : ANTHROPIC_API_KEY is not set');
    expect(t).toContain('? ollama (local, 127.0.0.1:11434) not checked');
    expect(t).not.toContain('GET /api/providers asks');
    expect(html).toMatch(/<button[^>]*>Check<\/button>/);
    expect(HEALTH.map(providerLine)).toEqual([
      '✓ OpenRouter ready',
      '✕ OpenAI unavailable: OPENAI_API_KEY is not set',
      '✕ Anthropic (Claude) unavailable: ANTHROPIC_API_KEY is not set',
      '? ollama (local, 127.0.0.1:11434) not checked',
    ]);
  });

  it('once /api/providers answers, the local server shows what it said', async () => {
    const f = vi.fn(async (_url: string) => new Response(JSON.stringify(CHECKED), { status: 200 }));
    vi.stubGlobal('fetch', f);
    const got = await checkProviders();
    expect(f.mock.calls[0][0]).toBe('/api/providers');
    const statuses = bestProviders(got, HEALTH);
    expect(statuses).toBe(got);
    const html = renderToStaticMarkup(<ProviderStatusLine statuses={statuses} loading={false} onCheck={() => {}} />);
    expect(text(html)).toContain('✓ ollama (local, 127.0.0.1:11434) ready');
    expect(html).toMatch(/<button[^>]*>Check again<\/button>/);
    expect(text(html)).not.toContain('not checked');
    const down = { ...HEALTH[3], checked: true, why: 'ollama (127.0.0.1:11434) did not answer within 3 s' };
    expect(providerLine(down)).toBe('✕ ollama (local, 127.0.0.1:11434) unavailable: ollama (127.0.0.1:11434) did not answer within 3 s');
  });

  it('the old single OpenRouter entry is gone from the modes line', () => {
    expect(modesLine({ open_available: false, claude: true, codex: true })).toBe('Claude Code · Codex');
    expect(modesLine({ open_available: false, claude: false, codex: false })).toBe('No agent CLI on PATH');
  });
});

describe('team editor', () => {
  it('an API-mode agent picks a provider (unavailable ones disabled, with why) and a listed model', () => {
    const a = agent({ provider: 'local:ollama', model: 'llama3.3' });
    const html = renderToStaticMarkup(<AgentFields agent={a} index={0} openOk list={CHECKED} patch={() => {}} />);
    expect(html).toContain('<option value="openai" disabled="">OpenAI (unavailable: OPENAI_API_KEY is not set)</option>');
    expect(html).toContain('<option value="openrouter">OpenRouter</option>');
    expect(html).toMatch(/<option value="local:ollama" selected="">ollama \(local, 127.0.0.1:11434\)<\/option>/);
    expect(html).toContain('list="team-models-0"');
    expect(html).toContain('<datalist id="team-models-0"><option value="llama3.3"></option><option value="qwen3:8b"></option></datalist>');
    expect(text(html)).toContain('2 models listed by ollama (local, 127.0.0.1:11434); or type any model id.');
  });

  it('choosing a provider sends it with its model; a CLI agent has no provider', () => {
    const patches: Partial<AgentSpec>[] = [];
    const tree = AgentFields({ agent: agent({ model: 'deepseek/x' }), index: 0, openOk: true, list: CHECKED, patch: (p) => patches.push(p) });
    find(tree, (e) => e.type === ProviderSelect)!.props.onChange!('local:ollama');
    expect(patches).toEqual([{ provider: 'local:ollama', model: null }]);

    const cli = renderToStaticMarkup(<AgentFields agent={agent({ kind: 'claude_code', model: 'sonnet' })} index={1} openOk list={CHECKED} patch={() => {}} />);
    expect(cli).not.toContain('Provider');
    expect(cli).not.toContain('<datalist');
  });

  it('a provider without a default asks for a model', () => {
    const html = renderToStaticMarkup(<AgentFields agent={agent({ provider: 'local:ollama' })} index={0} openOk list={CHECKED} patch={() => {}} />);
    expect(text(html)).toContain('Name a model for ollama (local, 127.0.0.1:11434)');
    expect(html).toContain('placeholder="model id (required)"');
    const def = renderToStaticMarkup(<AgentFields agent={agent()} index={0} openOk list={CHECKED} patch={() => {}} />);
    expect(def).toContain('placeholder="deepseek/deepseek-chat-v3.1 (default)"');
  });

  it('the team a run starts with never sends a blank provider', () => {
    const team = { id: 't', name: 't', agents: [
      agent({ id: 'a', provider: ' openai ', model: 'gpt-x' }),
      agent({ id: 'b', provider: '  ' }),
      agent({ id: 'c', kind: 'codex', provider: 'openai' }),
    ] };
    const sent = JSON.parse(JSON.stringify(teamForRun(team)));
    expect(sent.agents.map((a: AgentSpec) => a.provider)).toEqual(['openai', undefined, undefined]);
    expect('provider' in sent.agents[1]).toBe(false);
  });
});

describe('provider options', () => {
  it('keeps the current provider even when it is unavailable or not configured', () => {
    expect(providerOptions(HEALTH, 'openai').find((o) => o.id === 'openai')!.disabled).toBe(false);
    expect(providerOptions(HEALTH, 'openrouter').find((o) => o.id === 'openai')!.disabled).toBe(true);
    expect(providerOptions(HEALTH, 'local:gone').at(-1)).toEqual({ id: 'local:gone', text: 'local:gone (not configured)', disabled: false });
    expect(providerOptions(HEALTH, 'openrouter').find((o) => o.id === 'local:ollama')).toMatchObject({ disabled: false, text: 'ollama (local, 127.0.0.1:11434) (not checked)' });
    expect(modelProblem('local:gone', 'm', HEALTH)).toBe('local:gone is not configured on this arena');
    expect(modelProblem('openai', 'gpt-x', HEALTH)).toBe('OpenAI is unavailable: OPENAI_API_KEY is not set');
  });
});

describe('fork panel', () => {
  const agents = [agent({ model: 'deepseek/x' }), agent({ id: 'triage', name: 'Triage', kind: 'claude_code', model: 'sonnet' })];

  it('shows provider and model for an API-mode agent, and resets the model with the provider', () => {
    const edits: object[] = [];
    const props = { agent: agents[0], edit: {}, list: CHECKED, onEdit: (p: object) => edits.push(p) };
    const html = renderToStaticMarkup(<AgentEdit {...props} />);
    expect(html).toContain('<option value="anthropic" disabled="">Anthropic (Claude) (unavailable: ANTHROPIC_API_KEY is not set)</option>');
    expect(html).toContain('<option value="open" selected="">API mode</option>');
    find(AgentEdit(props), (e) => e.type === ProviderSelect)!.props.onChange!('local:ollama');
    expect(edits).toEqual([{ provider: 'local:ollama', model: '' }]);
    expect(renderToStaticMarkup(<AgentEdit {...props} agent={agents[1]} />)).not.toContain('Provider');
  });

  it('a provider change is always sent with its model', () => {
    expect(changedFields(agents, { resolver: { provider: 'local:ollama', model: 'llama3.3' } }))
      .toEqual({ resolver: { provider: 'local:ollama', model: 'llama3.3' } });
    expect(changedFields(agents, { resolver: { provider: 'local:ollama', model: '' } }))
      .toEqual({ resolver: { provider: 'local:ollama', model: '' } });
    // The agent's own provider (absent is openrouter) is no change; a CLI agent sends none.
    expect(changedFields(agents, { resolver: { provider: 'openrouter' } })).toEqual({});
    expect(changedFields(agents, { triage: { provider: 'openai', model: 'sonnet' } })).toEqual({});
    expect(changedFields(agents, { triage: { kind: 'open', provider: 'local:ollama', model: 'llama3.3' } }))
      .toEqual({ triage: { kind: 'open', provider: 'local:ollama', model: 'llama3.3' } });
  });

  it('blocks a fork that would leave an agent without a model or on an unavailable provider', () => {
    const noModel = changedFields(agents, { resolver: { provider: 'local:ollama', model: '' } });
    expect(applied(agents[0], noModel.resolver)).toMatchObject({ provider: 'local:ollama', model: null });
    expect(forkProblem(agents, noModel, CHECKED)).toBe('Resolver: Name a model for ollama (local, 127.0.0.1:11434)');
    const off = changedFields(agents, { resolver: { provider: 'openai', model: 'gpt-x' } });
    expect(forkProblem(agents, off, CHECKED)).toBe('Resolver: OpenAI is unavailable: OPENAI_API_KEY is not set');
    const ok = changedFields(agents, { resolver: { provider: 'local:ollama', model: 'llama3.3' } });
    expect(forkProblem(agents, ok, CHECKED)).toBeNull();
    expect(forkProblem(agents, { triage: { prompt: 'q' } }, CHECKED)).toBeNull();
  });
});

describe('model and cost per provider', () => {
  it('lists the provider next to the model', () => {
    expect(agentModelLine(agent())).toBe('deepseek/deepseek-chat-v3.1 (default) · OpenRouter');
    expect(agentModelLine(agent({ provider: 'anthropic', model: 'claude-x' }))).toBe('claude-x · Anthropic');
    expect(agentModelLine(agent({ provider: 'local:ollama', model: 'llama3.3' }))).toBe('llama3.3 · ollama (local)');
    expect(agentModelLine(agent({ kind: 'claude_code', model: 'sonnet' }))).toBe('sonnet · Claude Code');
  });

  it('OpenAI and Anthropic cost is unknown; a local server has no API charge', () => {
    expect(formatCost(null, null, true)).toBe('unknown');
    expect(formatCost(null, null)).toBe('tokens only');
    expect(formatCost(0, 'local (no API charge)')).toBe('no API charge');
    expect(formatCost(0.5, 'mixed')).toBe('$0.50 (mixed)');
  });
});

describe('status bar summary', () => {
  it('says how many providers are ready and, when none is, the reason they share', () => {
    expect(providerSummary([])).toBeNull();
    expect(providerSummary(HEALTH)).toEqual({ glyph: '✓', text: '1 of 4 ready, 1 not checked' });
    expect(providerSummary(CHECKED)).toEqual({ glyph: '✓', text: '2 of 4 ready' });
    const keyless = HEALTH.slice(0, 3).map((s) => ({ ...s, available: false, why: `${s.id.toUpperCase()}_API_KEY is not set` }));
    expect(providerSummary(keyless)).toEqual({ glyph: '✕', text: '0 of 3 ready: no API key set' });
    const down = [{ ...HEALTH[3], checked: true, why: 'did not answer' }];
    expect(providerSummary(down)).toEqual({ glyph: '✕', text: '0 of 1 ready: did not answer' });
  });
});
