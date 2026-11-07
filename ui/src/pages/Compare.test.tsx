import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { Rows, Side } from './Compare';
import type { CompareResponse, CompareSide } from '../types';

const text = (html: string) => html.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ');

const side: CompareSide = { status: 'passed', seconds: 1, tokens: 1, cost_usd: null, cost_source: null };
const cmp = (over: Partial<CompareResponse>): CompareResponse =>
  ({ a: side, b: side, first_divergent_seq: null, differing_rows: [], ...over });

describe('compare rows', () => {
  it('a part left out never reads as no differences', () => {
    const html = renderToStaticMarkup(<Rows data={cmp({ not_compared: ['plant'] })} />);
    expect(html).toContain('Not compared: plant (it keeps no saves)');
    expect(html).toContain('No differences in the parts compared.');
    expect(html).not.toContain('identical');
  });
  it('says identical only when every part was compared', () => {
    expect(renderToStaticMarkup(<Rows data={cmp({ not_compared: [] })} />)).toContain('The final worlds are identical.');
  });
});

describe('compare sides', () => {
  it('lists each agent with its model and provider; OpenAI cost is unknown', () => {
    const team = { id: 't', name: 't', agents: [
      { id: 'triage', name: 'Triage', kind: 'claude_code' as const, model: 'sonnet', prompt: '' },
      { id: 'resolver', name: 'Resolver', kind: 'open' as const, provider: 'openai', model: 'gpt-x', prompt: '' },
    ] };
    const html = text(renderToStaticMarkup(<Side id="abcdef0123" s={side} team={team} />));
    expect(html).toContain('Cost unknown');
    expect(html).toContain('Triage: sonnet · Claude Code');
    expect(html).toContain('Resolver: gpt-x · OpenAI');
    const local = { ...side, cost_usd: 0, cost_source: 'local (no API charge)' };
    expect(text(renderToStaticMarkup(<Side id="b" s={local} />))).toContain('Cost no API charge');
  });
});
