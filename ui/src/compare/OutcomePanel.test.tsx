import { renderToStaticMarkup } from 'react-dom/server';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, it } from 'vitest';
import ScenarioBuilder, { type ScenarioState } from '../scenario/ScenarioBuilder';
import { scenarioOptions } from '../scenario/fixtures';
import { initialSelection } from '../scenario/scenario';
import { FORK_SEQ, comparison, v1Detail, v2Detail } from '../world/fixtures';
import { TwinTile } from '../world/TwinTile';
import OutcomePanel from './OutcomePanel';

const text = (html: string) => html.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ');

describe('outcome panel', () => {
  it('shows each outcome for A and B, the better side, and the divergence strip', () => {
    const html = text(renderToStaticMarkup(
      <MemoryRouter><OutcomePanel a={{ detail: v1Detail, side: comparison.a }} b={{ detail: v2Detail, side: comparison.b }} first={FORK_SEQ} /></MemoryRouter>,
    ));
    expect(html).toContain('Valid plan every goal check passes ✕ not valid 8/9 ✓ valid 9/9 B better');
    expect(html).toContain('Escalations approval requests 3 3 same');
    expect(html).toContain(`first divergence · step ${FORK_SEQ}`);
    expect(html).toContain('Demo: Coordinated team v2');
  });
});

describe('scenario builder', () => {
  const state = (over: Partial<ScenarioState> = {}): ScenarioState => ({
    options: scenarioOptions, sel: initialSelection(scenarioOptions), error: null, toggle: () => {}, setParam: () => {}, scenario: undefined, problem: null, ...over,
  });
  it('lists the disruptions, the parameters of those on, and the preview', () => {
    const html = text(renderToStaticMarkup(<ScenarioBuilder s={state()} busy={false} />));
    expect(html).toContain('Usual courier loses capacity logistics');
    expect(html).toContain('Courier');
    expect(html).not.toContain('Extra days');
    expect(html).toContain('Tundra Medical Courier loses capacity after its first quote. The agents are not told.');
  });
  it('shows nothing when the benchmark offers no disruptions, and the problem that blocks starting', () => {
    expect(renderToStaticMarkup(<ScenarioBuilder s={state({ options: null })} busy={false} />)).toBe('');
    expect(text(renderToStaticMarkup(<ScenarioBuilder s={state({ problem: 'nope' })} busy={false} />))).toContain('⚠ nope');
  });
});

describe('twin tile', () => {
  it('carries a glyph and a word, and what changed', () => {
    const html = text(renderToStaticMarkup(<TwinTile item={{ id: 'CVX-2611', label: 'CVX-2611', status: 'on hold', abnormal: true, qty: 3600, unit: 'syr' }} change="quarantined → on hold" />));
    expect(html).toContain('CVX-2611 ■ on hold 3,600 syr ▸ quarantined → on hold');
  });
});
