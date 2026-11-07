import { useSyncExternalStore } from 'react';
import { THEME_KEY, type Theme, type ThemePref, parsePref, resolveTheme } from './themeModel';

const media = () => (typeof window !== 'undefined' && window.matchMedia ? window.matchMedia('(prefers-color-scheme: dark)') : null);

function readPref(): ThemePref {
  try {
    return parsePref(localStorage.getItem(THEME_KEY));
  } catch {
    return 'system';
  }
}

interface ThemeState { pref: ThemePref; resolved: Theme }
let state: ThemeState = { pref: 'system', resolved: 'light' };
const listeners = new Set<() => void>();

function apply(pref: ThemePref) {
  state = { pref, resolved: resolveTheme(pref, !!media()?.matches) };
  if (typeof document !== 'undefined') document.documentElement.dataset.theme = state.resolved;
  listeners.forEach((f) => f());
}

/** Reads the stored preference, applies it, and follows the system while the preference is "system". */
export function initTheme() {
  apply(readPref());
  media()?.addEventListener('change', () => apply(state.pref));
}

export function setThemePref(pref: ThemePref) {
  try {
    localStorage.setItem(THEME_KEY, pref);
  } catch {
    /* storage unavailable: the choice lasts for this page */
  }
  apply(pref);
}

const subscribe = (f: () => void) => { listeners.add(f); return () => { listeners.delete(f); }; };

export function useTheme(): ThemeState {
  return useSyncExternalStore(subscribe, () => state, () => state);
}
