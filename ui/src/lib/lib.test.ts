import { describe, expect, it } from 'vitest';
import { lineOptions, sparkOptions } from './chart';
import { isTyping, moveIn, moveIndex, navOf } from './keys';
import { clampWidth, nudge } from './split';

describe('keys', () => {
  it('maps arrows, j/k and Home/End to moves; sideways only when asked', () => {
    expect(['ArrowDown', 'j', 'ArrowUp', 'k', 'Home', 'End', 'x'].map((k) => navOf(k))).toEqual(['next', 'next', 'prev', 'prev', 'first', 'last', null]);
    expect(navOf('ArrowRight')).toBeNull();
    expect([navOf('ArrowRight', true), navOf('h', true)]).toEqual(['next', 'prev']);
  });
  it('moves inside the list and starts from the near end', () => {
    expect(moveIndex(null, 3, 'next')).toBe(0);
    expect(moveIndex(null, 3, 'prev')).toBe(2);
    expect(moveIndex(2, 3, 'next')).toBe(2);
    expect(moveIndex(0, 3, 'prev')).toBe(0);
    expect(moveIndex(1, 3, 'last')).toBe(2);
    expect(moveIndex(null, 0, 'next')).toBeNull();
    expect(moveIn([4, 7, 9], 7, 'next')).toBe(9);
    expect(moveIn([4, 7, 9], 5, 'next')).toBe(4);
  });
  it('leaves keys to text fields', () => {
    expect(isTyping({ tagName: 'INPUT' } as unknown as EventTarget)).toBe(true);
    expect(isTyping({ tagName: 'DIV', isContentEditable: false } as unknown as EventTarget)).toBe(false);
    expect(isTyping(null)).toBe(false);
  });
});

describe('split', () => {
  it('keeps the side panel inside its limits and leaves room for the main area', () => {
    const limits = { min: 300, max: 760 };
    expect(clampWidth(100, limits)).toBe(300);
    expect(clampWidth(900, limits)).toBe(760);
    expect(clampWidth(700, limits, 1000)).toBe(640);
    expect(clampWidth(700, limits, 500)).toBe(300);
  });
  it('nudges with the arrow keys, the panel being on the right', () => {
    expect(nudge(400, 'ArrowLeft', false)).toBe(416);
    expect(nudge(400, 'ArrowRight', true)).toBe(336);
    expect(nudge(400, 'Enter', false)).toBeNull();
  });
});

describe('chart options', () => {
  const c = { series: '#111', axis: '#222', grid: '#333', alarm: '#f00' };
  it('a sparkline has no axes, legend or cursor', () => {
    const o = sparkOptions(200, 28, c);
    expect([o.width, o.height, o.legend?.show, o.cursor?.show]).toEqual([200, 28, false, false]);
    expect(o.axes?.every((a) => a.show === false)).toBe(true);
    expect(o.series[1].stroke).toBe('#111');
  });
  it('a chart labels its time axis in seconds', () => {
    const o = lineOptions(400, 140, c, 'context');
    const values = o.axes![0].values as (u: unknown, v: number[]) => string[];
    expect(values(null, [0, 12.4])).toEqual(['0 s', '12 s']);
    expect(o.series[1].label).toBe('context');
  });
});
