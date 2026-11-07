import { useCallback } from 'react';
import { useSearchParams } from 'react-router-dom';

/** A `?step=` value as a step number, or null. */
export function parseStep(v: string | null): number | null {
  if (v == null || v.trim() === '') return null;
  const n = Number(v);
  return Number.isInteger(n) ? n : null;
}

/** A page's address with the selected step, so the next page opens at the same one. */
export const withStep = (path: string, step: number | null) => (step == null ? path : `${path}?step=${step}`);

type Next = number | null | ((s: number | null) => number | null);

/** The selected step, kept in the address (`?step=`): the run page, the process view and the world view share it. */
export function useStepParam(): [number | null, (next: Next) => void] {
  const [sp, setSp] = useSearchParams();
  const step = parseStep(sp.get('step'));
  const set = useCallback((next: Next) => {
    setSp((prev) => {
      const v = typeof next === 'function' ? next(parseStep(prev.get('step'))) : next;
      const out = new URLSearchParams(prev);
      if (v == null) out.delete('step');
      else out.set('step', String(v));
      return out;
    }, { replace: true });
  }, [setSp]);
  return [step, set];
}
