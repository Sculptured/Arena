import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import Timeline from './Timeline';
import { modelCallLine } from './Inspector';
import type { Team } from '../types';

const team: Team = { id: 't', name: 't', agents: [
  { id: 'resolver', name: 'Resolver', kind: 'open', provider: 'local:ollama', model: 'llama3.3', prompt: '' },
] };

describe('provider next to the model', () => {
  it('each lane shows its agent model and provider', () => {
    const html = renderToStaticMarkup(<Timeline lanes={[{ agent: 'resolver', steps: [] }]} team={team} selected={null} onSelect={() => {}} />);
    expect(html).toContain('<span class="lane-model">llama3.3 · ollama (local)</span>');
    expect(html).toContain('title="resolver · llama3.3 · ollama (local)"');
  });
  it('a model call names the provider that answered', () => {
    expect(modelCallLine({ provider: 'anthropic', request: { model: 'claude-x' } })).toBe('Anthropic · claude-x');
    expect(modelCallLine({ request: { model: 'deepseek/x' } })).toBe('OpenRouter · deepseek/x');
  });
});
