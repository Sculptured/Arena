// Figures as text: seconds, tokens, dollars and plain counts, one way everywhere.

export type Unit = 's' | 'tok' | 'usd' | 'n';

/** "3.0 s", "12.4k", "$0.56" (three decimals under ten cents), "2". */
export function fmt(x: number | null, unit: Unit): string {
  if (x == null) return '–';
  if (unit === 's') return `${x.toFixed(1)} s`;
  if (unit === 'usd') return `$${x.toFixed(x < 0.1 ? 3 : 2)}`;
  if (unit === 'tok') return x >= 10_000 ? `${(x / 1000).toFixed(1)}k` : `${Math.round(x)}`;
  return Number.isInteger(x) ? `${x}` : x.toFixed(2);
}

/** "45 s" under a minute and a half, else "3.1 min". */
export function duration(s: number | null): string {
  if (s == null) return '–';
  return s < 90 ? `${Math.round(s)} s` : `${(s / 60).toFixed(1)} min`;
}
