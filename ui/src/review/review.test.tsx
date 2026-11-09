import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { MemoryRouter } from 'react-router-dom';
import type { BranchDetail, Event } from '../types';
import { reviewPath } from './api';
import { modelName, smallPrint } from './present';
import { RunReviewBody } from './ReviewCard';
import { parseOn } from './setting';
import type { CompareReview, RunReview, StoredReview, Suggestion } from './types';
import { CompareBody, citeLabel } from './WhatMatters';

const text = (html: string) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;/g, "'").replace(/\s+/g, ' ');

const team = { id: 't', name: 'Team', agents: [
  { id: 'quality', name: 'Quality assurance', kind: 'claude_code' as const, model: 'haiku', prompt: 'Open the deviation.' },
  { id: 'production', name: 'Production planner', kind: 'claude_code' as const, model: 'haiku', prompt: 'Bring the next batch forward.' },
] };

const ev = (seq: number, kind: Event['kind'], agent: string | null): Event =>
  ({ branch_id: 'b', seq, at: '2026-11-08T00:00:00Z', agent, kind, payload: {} });

const events: Event[] = [ev(1, 'run_start', null), ev(3, 'agent_start', 'quality'), ev(5, 'tool_call', 'quality'), ev(212, 'agent_start', 'production'), ev(215, 'tool_call', 'production'), ev(304, 'check', null)];

const detail = { id: 'b', run_id: 'r', team, events, status: 'failed' } as unknown as BranchDetail;

const tip = (over: Partial<Suggestion>): Suggestion => ({ change: 'c', why: 'w', kind: 'prompt', ...over });

describe('review presentation', () => {
  it('says who reviewed, what it cost and how long it took', () => {
    const s = { model: 'claude_code:sonnet', cost_usd: 0.0412, cost_source: 'api_equivalent_estimate', tokens: 30000, seconds: 41.4 } as StoredReview;
    expect(smallPrint(s)).toBe('Reviewed by Claude Code sonnet · $0.041 (API-equivalent) · 41 s');
    expect(modelName('openrouter:deepseek/deepseek-chat-v3.1')).toBe('openrouter deepseek/deepseek-chat-v3.1');
    expect(smallPrint({ ...s, cost_usd: null, seconds: null })).toBe('Reviewed by Claude Code sonnet · 30,000 tokens');
  });
  it('is on unless this browser turned it off, and names its routes', () => {
    expect([parseOn(null), parseOn('on'), parseOn('off')]).toEqual([true, true, false]);
    expect(reviewPath({ kind: 'run', branch: 'b 1' })).toBe('/api/branches/b%201/review');
    expect(reviewPath({ kind: 'suites', a: 'x', b: 'y' })).toBe('/api/suites/compare/review?a=x&b=y');
  });
});

describe('review cards', () => {
  const review: RunReview = {
    verdict: 'The next batch was never brought forward.',
    went_wrong: [{ title: 'Schedule change rejected', why: 'The planner asked the wrong desk.', steps: [215], severity: 'goal' },
      { title: 'Unverified', why: 'w', steps: [], severity: 'efficiency', unverified: true }],
    suggestions: [tip({ agent: 'production', change: 'Route schedule changes to QA', prompt_text: 'QA decides schedule changes.' }), tip({ kind: 'process', change: 'Batch shipments' })],
    went_well: ['No safety check failed'],
    confidence: 'high',
  };
  it('shows the verdict, steps and suggestions, and never offers a rewind', () => {
    const html = text(renderToStaticMarkup(<RunReviewBody r={review} d={detail} onStep={() => {}} />));
    for (const s of ['The next batch was never brought forward.', 'Where it went wrong', 'Goal', 'Step 215', 'Suggestions', 'Production planner', 'What went well', '✓ No safety check failed', "its steps weren't in the run"]) {
      expect(html).toContain(s);
    }
    expect(html).not.toContain('Rewind');
  });
  it('shows what matters between two runs with links to each side\'s steps', () => {
    const r: CompareReview = { headline: 'B opened the deviation.', which_is_better: 'b', why: 'Fewer failed checks.',
      differences: [{ title: 'Deviation', a: 'Failed at step 110', b: 'Created', why_it_matters: 'A goal check', steps_a: [110], steps_b: [34] }] };
    const html = renderToStaticMarkup(<MemoryRouter><CompareBody r={r} hrefOf={(side, c) => `/runs/r/${side}?step=${c}`} /></MemoryRouter>);
    expect(text(html)).toContain('B did better');
    expect(html).toContain('href="/runs/r/a?step=110"');
    expect(html).toContain('href="/runs/r/b?step=34"');
    expect(citeLabel('1a2b3c4d5e6f')).toBe('Run 1a2b3c4d');
  });
});
