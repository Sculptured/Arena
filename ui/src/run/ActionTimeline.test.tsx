import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import ActionTimeline from './ActionTimeline';
import { modelCallLine } from './Inspector';
import { buildLanes, spanOf } from '../control/lanes';
import { events, team as fixtureTeam } from '../control/fixtures';
import type { Team } from '../types';

const text = (html: string) => html.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ');

const team: Team = { id: 't', name: 't', agents: [
  { id: 'resolver', name: 'Resolver', kind: 'open', provider: 'local:ollama', model: 'llama3.3', prompt: '' },
] };

describe('action timeline', () => {
  it('each lane shows its agent, model and provider', () => {
    const html = renderToStaticMarkup(<ActionTimeline lanes={[{ id: 'resolver', name: 'resolver', marks: [] }]} team={team} span={null} selected={null} onSelect={() => {}} models onModels={() => {}} />);
    expect(text(html)).toContain('Resolver llama3.3 · ollama (local)');
    expect(html).toContain('title="resolver · llama3.3 · ollama (local)"');
  });
  it('labels marks by system kind and read/write, and says when an action was refused', () => {
    const lanes = buildLanes(events, fixtureTeam);
    const html = renderToStaticMarkup(<ActionTimeline lanes={lanes} team={fixtureTeam} span={spanOf(lanes)} selected={9} onSelect={() => {}} models onModels={() => {}} />);
    expect(html).toContain('aria-label="step 6 · LIMS read');
    expect(html).toContain('aria-label="step 9 · ERP write · hold_batch: hold_batch refused: no approval for CVX-2611"');
    expect(html).toContain('aria-pressed="true"');
    expect(html).toContain('aria-label="What the marks mean"');
  });
  it('a model call names the provider that answered', () => {
    expect(modelCallLine({ provider: 'anthropic', request: { model: 'claude-x' } })).toBe('Anthropic · claude-x');
    expect(modelCallLine({ request: { model: 'deepseek/x' } })).toBe('OpenRouter · deepseek/x');
  });
});
