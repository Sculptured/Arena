import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { Chart } from './BestSoFar';
import type { Study } from './candidates';

const study: Study = {
  objective: { label: 'Line 2 OEE', unit: '%', better: 'higher' },
  target: 85, verified: 87.5,
  candidates: [
    { seq: 7, n: 1, value: 76.9, ci: 0.8, feasible: true, changes: '', best: 76.9 },
    { seq: 10, n: 2, value: 88.1, ci: 0.9, feasible: false, changes: 'line2.batch_size 400', best: 76.9 },
    { seq: 13, n: 3, value: 86.2, ci: null, feasible: true, changes: 'line2.crew 2/2/1/1', best: 86.2 },
  ],
};

describe('best so far', () => {
  const html = renderToStaticMarkup(<Chart s={study} width={600} selected={13} onSelect={() => {}} />);
  it('draws a dot per candidate with what it changed, and marks the one that missed a limit', () => {
    expect(html).toContain('<title>Simulation 1 · 76.9% ± 0.8\nToday&#x27;s config</title>');
    expect(html).toContain('Simulation 2 · 88.1% ± 0.9\nline2.batch_size 400\nMissed a limit');
    expect(html.match(/r="3.5"/g)).toHaveLength(3);
  });
  it('labels the target and the verified value, and rings the selected step', () => {
    expect(html).toContain('Target 85%');
    expect(html).toContain('Verified 87.5%');
    expect(html.match(/stroke="var\(--accent\)"/g)).toHaveLength(1);
  });
});
