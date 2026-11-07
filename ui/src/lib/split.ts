// The resizable side panel's width: kept inside its limits and remembered per panel.

export interface Limits {
  min: number;
  max: number;
}

/** A width inside the limits, and never more than `room` minus a minimum for the main area. */
export function clampWidth(width: number, limits: Limits, room = Infinity, mainMin = 360): number {
  const max = Math.min(limits.max, room - mainMin);
  return Math.round(Math.max(limits.min, Math.min(Math.max(limits.min, max), width)));
}

/** The width after a keyboard nudge on the handle: ←/→ move 16 px (64 px with shift). The panel is on the right. */
export function nudge(width: number, key: string, shift: boolean): number | null {
  const step = shift ? 64 : 16;
  if (key === 'ArrowLeft') return width + step;
  if (key === 'ArrowRight') return width - step;
  return null;
}

export function readWidth(key: string, fallback: number): number {
  try {
    const v = Number(localStorage.getItem(key));
    return Number.isFinite(v) && v > 0 ? v : fallback;
  } catch {
    return fallback;
  }
}

export function storeWidth(key: string, width: number): void {
  try {
    localStorage.setItem(key, String(width));
  } catch {
    /* storage unavailable: the width lasts for this page */
  }
}
