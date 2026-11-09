import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { ConnectView, type TestState } from './ConnectPage';
import { type ConnectForm, emptyForm } from './connect';

const draw = (form: ConnectForm, test: TestState = { state: 'idle' }) =>
  renderToStaticMarkup(<ConnectView form={form} test={test} onChange={() => {}} onSubmit={() => {}} onCancel={() => {}} />);

const filled: ConnectForm = { ...emptyForm(), url: 'http://localhost:8800/agent', headers: 'Authorization: Bearer sk-live-123' };

describe('Connect your agent', () => {
  it('starts empty: HTTP chosen, the URL and headers, Test connection waiting for a URL', () => {
    const html = draw(emptyForm());
    expect(html).toContain('Connect your agent');
    expect(html).toMatch(/aria-checked="true"[^>]*>HTTP</);
    expect(html).toContain('placeholder="https://agent.example.com/arena"');
    expect(html).toMatch(/type="submit" disabled=""><svg.*?<\/svg>Test connection<\/button>/);
    expect(html).not.toContain('⚠');
  });
  it('masks the header values once typed, and never echoes them outside their box', () => {
    const html = draw(filled, { state: 'ok', agent: { id: 'p', name: 'Plant planner', kind: 'http', model: null, prompt: '', url: filled.url, headers: { Authorization: { secret: 'r' } } }, ms: 142.4 });
    expect(html).toContain('[-webkit-text-security:disc]');
    expect(html.split('sk-live-123')).toHaveLength(2); // the textarea's own value only
  });
  it('after a good test: the agent\'s name and round trip, and Use this agent', () => {
    const html = draw(filled, { state: 'ok', agent: { id: 'p', name: 'Plant planner', kind: 'http', model: null, prompt: '' }, ms: 142.4 });
    expect(html).toContain('✓ Connected to <span class="font-medium">Plant planner</span>');
    expect(html).toContain(' · 142 ms');
    expect(html).toContain('Use this agent');
    expect(html).not.toContain('Test connection');
  });
  it('a failed test says why in alarm ink, and a bad header line before any test', () => {
    expect(draw(filled, { state: 'error', error: 'connection refused' })).toMatch(/text-alarm[^>]*>⚠ connection refused</);
    expect(draw({ ...filled, headers: 'Bearer' })).toContain('⚠ Header line 1 isn&#x27;t “Name: value”.');
    expect(draw(filled, { state: 'testing' })).toContain('Testing…');
  });
  it('Bedrock asks for a model and a region (us-east-1 by default)', () => {
    const html = draw(emptyForm('bedrock'));
    expect(html).toMatch(/aria-checked="true"[^>]*>Bedrock</);
    expect(html).toContain('value="us-east-1"');
    expect(html).toContain('placeholder="anthropic.claude-3-5-haiku-20241022-v1:0"');
  });
});
