// Light, dark, or the system's choice: the preference, and the theme it resolves to.

export type ThemePref = 'system' | 'light' | 'dark';
export type Theme = 'light' | 'dark';

export const THEME_KEY = 'arena.theme';

export function resolveTheme(pref: ThemePref, systemDark: boolean): Theme {
  if (pref === 'system') return systemDark ? 'dark' : 'light';
  return pref;
}

/** The next preference when the toggle is pressed: system → light → dark → system. */
export function nextPref(pref: ThemePref): ThemePref {
  return pref === 'system' ? 'light' : pref === 'light' ? 'dark' : 'system';
}

export function parsePref(v: string | null): ThemePref {
  return v === 'light' || v === 'dark' ? v : 'system';
}
