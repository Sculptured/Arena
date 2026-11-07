// Keyboard navigation shared by tables, the timeline and the palette: which way a key moves.

export type Nav = 'next' | 'prev' | 'first' | 'last';

const KEYS: Record<string, Nav> = {
  ArrowDown: 'next', j: 'next', ArrowUp: 'prev', k: 'prev', Home: 'first', End: 'last',
};
const SIDEWAYS: Record<string, Nav> = { ArrowRight: 'next', l: 'next', ArrowLeft: 'prev', h: 'prev' };

/** The move a key makes in a list (↑/↓, j/k, Home/End); with `sideways`, ←/→ and h/l too. */
export function navOf(key: string, sideways = false): Nav | null {
  return KEYS[key] ?? (sideways ? SIDEWAYS[key] ?? null : null);
}

/** The index after a move in a list of `len` items; from nothing, next starts at the top and prev at the bottom. */
export function moveIndex(index: number | null, len: number, nav: Nav): number | null {
  if (len === 0) return null;
  if (nav === 'first') return 0;
  if (nav === 'last') return len - 1;
  if (index == null) return nav === 'next' ? 0 : len - 1;
  return Math.min(len - 1, Math.max(0, index + (nav === 'next' ? 1 : -1)));
}

/** The item after a move in an ordered list of ids (steps, rows); the current one may be absent. */
export function moveIn<T>(order: T[], current: T | null, nav: Nav): T | null {
  const i = current == null ? -1 : order.indexOf(current);
  const next = moveIndex(i < 0 ? null : i, order.length, nav);
  return next == null ? null : order[next];
}

/** True when keys belong to what has focus: a text field, a select, an editable element. */
export function isTyping(target: EventTarget | null): boolean {
  const t = target as HTMLElement | null;
  if (!t || typeof t.tagName !== 'string') return false;
  return /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName) || t.isContentEditable === true;
}

/** True when a key event carries a modifier other than shift (left to the browser and the palette). */
export const hasModifier = (e: { metaKey: boolean; ctrlKey: boolean; altKey: boolean }) => e.metaKey || e.ctrlKey || e.altKey;
