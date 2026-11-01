import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { ForkActions } from './ForkPanel';
import { RestoreChoice, blockedReason, loadPlan } from './RestoreChoice';

const REFUSAL = "the part isaac can't be restored exactly; choose an approximate restore";

function answer(status: number, body: unknown) {
  const f = vi.fn(async (_url: string) => new Response(JSON.stringify(body), { status }));
  vi.stubGlobal('fetch', f);
  return f;
}

afterEach(() => vi.unstubAllGlobals());

/** The text a reader sees in server-rendered markup. */
const text = (html: string) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;/g, "'").replace(/\s+/g, ' ');

describe('restore choice', () => {
  it('rerun panel shows per-part plan', async () => {
    const f = answer(200, { parts: [
      { part: 'retail', how: 'save' },
      { part: 'plant', how: 'replay', estimate_seconds: 40 },
    ] });
    const plan = await loadPlan('b1', 5, 'approximate');
    expect(f.mock.calls[0][0]).toBe('/api/branches/b1/restore-plan?at_seq=5&restore=approximate');
    const html = renderToStaticMarkup(<RestoreChoice mode="exact" onMode={() => {}} plan={plan} />);
    expect(text(html)).toContain('retail: from its save');
    expect(text(html)).toContain('plant: replayed from the start, about 40 s');
    expect(html).toMatch(/<input [^>]*checked="" value="exact"/);
    expect(blockedReason(plan, false)).toBeNull();
  });

  it('exact refusal disables start with reason', async () => {
    answer(409, { error: REFUSAL });
    const plan = await loadPlan('b1', 5, 'exact');
    expect(plan).toEqual({ state: 'refused', reason: REFUSAL });
    expect(text(renderToStaticMarkup(<RestoreChoice mode="exact" onMode={() => {}} plan={plan} />))).not.toContain(REFUSAL);
    const html = renderToStaticMarkup(<ForkActions blocked={blockedReason(plan, false)} busy={false} changes={0} onGo={() => {}} />);
    expect(html).toMatch(/<button[^>]*disabled=""/);
    expect(text(html)).toContain(REFUSAL);
  });

  it('other plan failures are shown but do not block the rerun', async () => {
    answer(500, { error: 'boom' });
    const plan = await loadPlan('b1', 5, 'exact');
    expect(plan).toEqual({ state: 'failed', error: 'boom' });
    expect(blockedReason(plan, false)).toBeNull();
    expect(blockedReason(plan, true)).toBe('Wait for this run to finish before forking.');
  });
});
