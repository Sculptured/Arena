import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { Rows } from './Compare';
import type { CompareResponse, CompareSide } from '../types';

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
    expect(renderToStaticMarkup(<Rows data={cmp({ not_compared: [] })} />)).toContain('The final databases are identical.');
  });
});
