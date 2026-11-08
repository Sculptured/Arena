// The timeline's mouse zoom and scroll: Ctrl/⌘ + wheel (and a trackpad pinch, which browsers send
// as a wheel with Ctrl) zooms around the cursor, the wheel scrolls sideways when there is more to
// see, and a drag on empty space pans. Pure: the component only measures and applies.
import { TRACK_PAD } from './timelineLayout';

export const MIN_ZOOM = 1;
export const MAX_ZOOM = 64;

/** How zoomed in the track is (1: the run fits) and how far it is scrolled, in track pixels. */
export interface View {
  zoom: number;
  scroll: number;
}

export interface WheelInput {
  deltaX: number;
  deltaY: number;
  /** 0 pixels, 1 lines, 2 pages (`WheelEvent.deltaMode`). */
  deltaMode: number;
  ctrlKey: boolean;
  metaKey: boolean;
}

export const clampZoom = (z: number) => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z));

/** The track's width at `zoom` for a viewport `viewW` wide; never under `minW`. */
export const trackWidth = (viewW: number, zoom: number, minW = 0) => Math.max(minW, viewW * zoom);

export const clampScroll = (scroll: number, trackW: number, viewW: number) => Math.min(Math.max(0, scroll), Math.max(0, trackW - viewW));

/** The scroll that shows the end of the track (the newest step) at the right edge. */
export const endScroll = (trackW: number, viewW: number) => Math.max(0, trackW - viewW);

/** A wheel delta in pixels, whatever its unit. */
export const deltaPx = (delta: number, mode: number) => (mode === 1 ? delta * 16 : mode === 2 ? delta * 400 : delta);

/** The zoom after a Ctrl/⌘ wheel move: up (negative) zooms in, smoothly. */
export const wheelZoom = (zoom: number, deltaY: number, deltaMode: number) => clampZoom(zoom * Math.exp(-deltaPx(deltaY, deltaMode) * 0.0025));

/** Zooms to `next`, keeping the time under the cursor (`cursor` px from the viewport's left) where it is. */
export function zoomAround(view: View, next: number, cursor: number, viewW: number, minW = 0): View {
  const zoom = clampZoom(next);
  const w0 = trackWidth(viewW, view.zoom, minW);
  const w1 = trackWidth(viewW, zoom, minW);
  const f = (view.scroll + cursor - TRACK_PAD) / Math.max(1, w0 - 2 * TRACK_PAD);
  const x1 = TRACK_PAD + f * (w1 - 2 * TRACK_PAD);
  return { zoom, scroll: clampScroll(x1 - cursor, w1, viewW) };
}

/**
 * What a wheel event over the track does: zoom (Ctrl/⌘ or a pinch), scroll sideways when the
 * track is wider than its viewport, or nothing (null), so the page scrolls as usual.
 */
export function onWheel(e: WheelInput, view: View, cursor: number, viewW: number, minW = 0): View | null {
  if (e.ctrlKey || e.metaKey) return zoomAround(view, wheelZoom(view.zoom, e.deltaY, e.deltaMode), cursor, viewW, minW);
  const trackW = trackWidth(viewW, view.zoom, minW);
  if (trackW <= viewW + 1) return null;
  const sideways = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY;
  const scroll = clampScroll(view.scroll + deltaPx(sideways, e.deltaMode), trackW, viewW);
  return scroll === view.scroll ? null : { ...view, scroll };
}

/** A zoom button: twice as close (or as far), around the middle of the viewport. */
export const stepZoom = (view: View, dir: 1 | -1, viewW: number, minW = 0) => zoomAround(view, view.zoom * 2 ** dir, viewW / 2, viewW, minW);

/**
 * The view that keeps the same moments on screen after the run's span changes from `from` to `to`
 * (a live run's newest event): the time at the left edge and the pixels per millisecond stay put,
 * so a user zoomed in away from the end sees nothing move. At Fit (zoom 1) the run keeps fitting.
 */
export function holdTimeWindow(view: View, from: { start: number; end: number }, to: { start: number; end: number }, viewW: number, minW = 0): View {
  const len0 = from.end - from.start, len1 = to.end - to.start;
  if (view.zoom <= MIN_ZOOM || len0 <= 0 || len1 <= 0 || viewW <= 0) return view;
  const w0 = trackWidth(viewW, view.zoom, minW);
  const pxPerMs = (w0 - 2 * TRACK_PAD) / len0;
  const leftAt = from.start + (view.scroll - TRACK_PAD) / pxPerMs;
  const zoom = clampZoom((pxPerMs * len1 + 2 * TRACK_PAD) / viewW);
  const w1 = trackWidth(viewW, zoom, minW);
  const scroll = TRACK_PAD + (leftAt - to.start) * ((w1 - 2 * TRACK_PAD) / len1);
  return { zoom, scroll: clampScroll(scroll, w1, viewW) };
}
