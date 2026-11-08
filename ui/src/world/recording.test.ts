import { describe, expect, it } from 'vitest';
import type { Event } from '../types';
import { arenaSecondAt, clockText } from './recording';

const ev = (seq: number, payload: unknown) => ({ seq, kind: 'tool_call', payload } as unknown as Event);

describe('the plant recording follows arena time', () => {
  it('takes the step\'s own clock, else the last before it', () => {
    const events = [ev(1, {}), ev(2, { clock_ms: 1500 }), ev(3, { name: 'x' }), ev(4, { clock_ms: 64000 })];
    expect(arenaSecondAt(events, 1)).toBeNull();
    expect(arenaSecondAt(events, 3)).toBe(1.5);
    expect(arenaSecondAt(events, 4)).toBe(64);
    expect(arenaSecondAt(events, null)).toBeNull();
    expect(clockText(64.9)).toBe('1:04');
  });
});
