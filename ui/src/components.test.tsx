import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { ApproximateMark, ago, diskFill, diskLine } from './components';

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

describe('disk fill and age', () => {
  it('says how full the saves are and when new runs are refused', () => {
    expect(diskFill(health(GB, 4 * GB))).toEqual({ fraction: 0.25, full: false });
    expect(diskFill(health(5 * GB, 4 * GB))).toEqual({ fraction: 1, full: true });
    expect(diskFill(null)).toBeNull();
  });
  it('says how long ago in words', () => {
    const now = Date.parse('2026-11-06T12:00:00Z');
    expect(ago('2026-11-06T11:59:30Z', now)).toBe('just now');
    expect(ago('2026-11-06T11:15:00Z', now)).toBe('45 min ago');
    expect(ago('2026-11-06T09:00:00Z', now)).toBe('3 h ago');
    expect(ago('not a date', now)).toBe('not a date');
  });
});
