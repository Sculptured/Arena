import { useCallback, useEffect, useRef, useState } from 'react';
import { type Target, getReview, startReview } from './api';
import type { CompareReview, RunReview, StoredReview } from './types';

/** How often a running review is asked about. */
export const POLL_MS = 3000;

const keyOf = (t: Target | null) => (t ? JSON.stringify(t) : '');

/**
 * A target's review: read when shown, started when `auto` and there is none yet, followed while
 * it runs. Nothing is asked while `enabled` is false.
 */
export function useReview<R extends RunReview | CompareReview>(target: Target | null, enabled: boolean, auto: boolean) {
  const [stored, setStored] = useState<StoredReview<R> | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const key = keyOf(target);
  const current = useRef(key);
  const land = useCallback((k: string, r: StoredReview<R> | null) => {
    if (current.current === k) { setStored(r); setLoaded(true); setError(null); }
  }, []);
  const fail = useCallback((k: string, e: unknown) => {
    if (current.current === k) { setError(e instanceof Error ? e.message : String(e)); setLoaded(true); }
  }, []);
  const start = useCallback((again = false) => {
    if (!target) return;
    const k = keyOf(target);
    setError(null);
    startReview<R>(target, again).then((r) => land(k, r), (e) => fail(k, e));
  }, [key, land, fail]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    current.current = key;
    setStored(null); setLoaded(false); setError(null);
    if (!target || !enabled) return;
    getReview<R>(target).then((r) => {
      land(key, r);
      if (!r && auto) start();
    }, (e) => fail(key, e));
  }, [key, enabled, auto]); // eslint-disable-line react-hooks/exhaustive-deps
  const running = stored?.status === 'running';
  useEffect(() => {
    if (!running || !target) return;
    const t = setInterval(() => { getReview<R>(target).then((r) => land(key, r), () => {}); }, POLL_MS);
    return () => clearInterval(t);
  }, [running, key]); // eslint-disable-line react-hooks/exhaustive-deps
  return { stored, loaded, error, start };
}
