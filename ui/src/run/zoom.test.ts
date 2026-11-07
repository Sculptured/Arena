import { describe, expect, it } from 'vitest';
import { TRACK_PAD, layoutTimeline } from './timelineLayout';
import { MAX_ZOOM, clampScroll, endScroll, onWheel, stepZoom, wheelZoom, zoomAround } from './zoom';

const wheel = (deltaY: number, mods: { ctrlKey?: boolean; metaKey?: boolean; deltaX?: number } = {}) =>
  ({ deltaX: mods.deltaX ?? 0, deltaY, deltaMode: 0, ctrlKey: !!mods.ctrlKey, metaKey: !!mods.metaKey });

/** Where track pixel `x` is as a fraction of the time span, at a track `w` wide. */
const fraction = (x: number, w: number) => (x - TRACK_PAD) / (w - 2 * TRACK_PAD);

describe('timeline zoom', () => {
  it('keeps the time under the cursor in place', () => {
    const viewW = 1000;
    const before = { zoom: 2, scroll: 300 };
    const after = zoomAround(before, 4, 250, viewW);
    expect(after.zoom).toBe(4);
    const t0 = fraction(before.scroll + 250, viewW * 2);
    const t1 = fraction(after.scroll + 250, viewW * 4);
    expect(t1).toBeCloseTo(t0, 6);
  });

  it('clamps the zoom and the scroll to the track', () => {
    expect(zoomAround({ zoom: 1, scroll: 0 }, 0.2, 500, 1000)).toEqual({ zoom: 1, scroll: 0 });
    expect(zoomAround({ zoom: 32, scroll: 0 }, 999, 0, 1000).zoom).toBe(MAX_ZOOM);
    expect(clampScroll(-5, 2000, 1000)).toBe(0);
    expect(clampScroll(5000, 2000, 1000)).toBe(1000);
    expect(endScroll(3000, 1000)).toBe(2000);
    expect(endScroll(800, 1000)).toBe(0);
  });

  it('zooms with Ctrl or ⌘ and a pinch, in when the wheel goes up', () => {
    expect(wheelZoom(2, -100, 0)).toBeGreaterThan(2);
    expect(wheelZoom(2, 100, 0)).toBeLessThan(2);
    expect(wheelZoom(2, -3, 1)).toBeCloseTo(wheelZoom(2, -48, 0));
    expect(onWheel(wheel(-100, { ctrlKey: true }), { zoom: 1, scroll: 0 }, 500, 1000)!.zoom).toBeGreaterThan(1);
    expect(onWheel(wheel(-100, { metaKey: true }), { zoom: 1, scroll: 0 }, 500, 1000)!.zoom).toBeGreaterThan(1);
  });

  it('scrolls sideways only when there is more to see, leaving the page alone otherwise', () => {
    expect(onWheel(wheel(120), { zoom: 1, scroll: 0 }, 500, 1000)).toBeNull();
    expect(onWheel(wheel(120), { zoom: 2, scroll: 0 }, 500, 1000)).toEqual({ zoom: 2, scroll: 120 });
    expect(onWheel(wheel(5, { deltaX: -80 }), { zoom: 2, scroll: 100 }, 500, 1000)).toEqual({ zoom: 2, scroll: 20 });
    // At the end already: nothing to scroll, so the page may.
    expect(onWheel(wheel(50), { zoom: 2, scroll: 1000 }, 500, 1000)).toBeNull();
  });

  it('zoom buttons double around the middle', () => {
    const v = stepZoom({ zoom: 1, scroll: 0 }, 1, 1000);
    expect(v.zoom).toBe(2);
    expect(fraction(v.scroll + 500, 2000)).toBeCloseTo(0.5, 6);
    expect(stepZoom(v, -1, 1000)).toEqual({ zoom: 1, scroll: 0 });
  });
});

describe('pause bands', () => {
  it('draw each pause where it falls on the track', () => {
    const span = { start: 0, end: 10_000 };
    const lanes = [{ id: 'a', name: 'A', marks: [] }];
    const width = 1000 + 2 * TRACK_PAD;
    const l = layoutTimeline(lanes, span, width, true, [{ start: 2000, end: 5000, open: false }, { start: 9000, end: 10_000, open: true }]);
    expect(l.bands).toEqual([{ x1: TRACK_PAD + 200, x2: TRACK_PAD + 500, open: false }, { x1: TRACK_PAD + 900, x2: TRACK_PAD + 1000, open: true }]);
  });
});
