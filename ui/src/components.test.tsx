import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { ApproximateMark, diskLine } from './components';

const GB = 2 ** 30;
const health = (bytes: number, limit = 5 * GB) => ({
  open_available: true, claude: false, codex: false, disk: { bytes, limit_bytes: limit },
});

describe('diskLine', () => {
  it('shows the saves used and the limit in GB', () => {
    expect(diskLine(health(0.4 * GB))).toBe('Saves 0.4 / 5 GB');
    expect(diskLine(health(12 * GB, 20 * GB))).toBe('Saves 12 / 20 GB');
  });
  it('is empty without disk info', () => {
    expect(diskLine(null)).toBe('');
    expect(diskLine({ open_available: true, claude: false, codex: false })).toBe('');
  });
});

describe('ApproximateMark', () => {
  it('approximate branch is labelled', () => {
    const html = renderToStaticMarkup(<ApproximateMark of={{ restored: [{ part: 'retail', how: 'save' }, { part: 'plant', how: 'approximate' }] }} />);
    expect(html).toContain('>Approximate restore</span>');
    expect(html).toContain('title="retail: from its save\nplant: approximate"');
    expect(renderToStaticMarkup(<ApproximateMark of={{ restored: [{ part: 'plant', how: 'replay' }] }} />)).toBe('');
    expect(renderToStaticMarkup(<ApproximateMark of={{}} />)).toBe('');
  });
});
