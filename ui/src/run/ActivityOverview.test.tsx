import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import ActivityOverview from './ActivityOverview';
import type { Step } from './timeline';

const step = (seq: number): Step => ({ seq, kind: 'tool', label: 'purchase', wrote: true, snapshot: true, inherited: false });

describe('shared event order', () => {
  it('places agents on one sequence axis and preserves the selected event', () => {
    const html = renderToStaticMarkup(<ActivityOverview lanes={[
      { agent: 'planning', steps: [step(10), step(30)] },
      { agent: 'procurement', steps: [step(20)] },
    ]} selected={20} onSelect={() => {}} />);
    expect(html).toContain('left:50%');
    expect(html).toContain('aria-label="procurement, step 20: purchase" aria-pressed="true"');
    expect(html).toContain('not time');
  });
  it('does not fabricate a scale for an empty run', () => {
    expect(renderToStaticMarkup(<ActivityOverview lanes={[]} selected={null} onSelect={() => {}} />))
      .toContain('Waiting for recorded activity');
  });
});
