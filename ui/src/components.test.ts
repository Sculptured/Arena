import { describe, expect, it } from 'vitest';
import { diskLine } from './components';

const GB = 2 ** 30;
const health = (bytes: number, limit = 5 * GB) => ({
  open_available: true, claude: false, codex: false, disk: { databases: 3, bytes, limit_bytes: limit },
});

describe('diskLine', () => {
  it('shows used and limit in GB', () => {
    expect(diskLine(health(0.4 * GB))).toBe('DB 0.4 / 5 GB');
    expect(diskLine(health(12 * GB, 20 * GB))).toBe('DB 12 / 20 GB');
  });
  it('is empty without disk info', () => {
    expect(diskLine(null)).toBe('');
    expect(diskLine({ open_available: true, claude: false, codex: false })).toBe('');
  });
});
