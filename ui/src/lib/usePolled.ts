import { useCallback, useEffect, useRef, useState } from 'react';

/**
 * Loads `load()` when `key` changes, and again every `ms` while `again(value)` holds (something
 * is still running). Keeps the last value through a failed read, with the error beside it.
 */
export function usePolled<T>(load: () => Promise<T>, again: (v: T) => boolean, ms: number, key: string) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);
  const fns = useRef({ load, again });
  fns.current = { load, again };
  useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const go = () => fns.current.load().then((v) => {
      if (!alive) return;
      setData(v); setError(null);
      if (fns.current.again(v)) timer = setTimeout(go, ms);
    }).catch((e) => alive && setError(String(e?.message ?? e)));
    go();
    return () => { alive = false; clearTimeout(timer); };
  }, [key, tick, ms]);
  const reload = useCallback(() => setTick((t) => t + 1), []);
  return { data, error, reload };
}
