import { useEffect, useState, useSyncExternalStore } from 'react';
import { reviewSettings } from './api';
import type { ReviewSettings } from './types';

/** This browser's choice (⋯ → AI review): on unless turned off. */
export const REVIEW_KEY = 'arena.review';

/** The stored choice read as on or off: anything but "off" is on. */
export const parseOn = (v: string | null) => v !== 'off';

function readOn(): boolean {
  try {
    return parseOn(localStorage.getItem(REVIEW_KEY));
  } catch {
    return true;
  }
}

let on = readOn();
const listeners = new Set<() => void>();

export function setReviewOn(next: boolean) {
  on = next;
  try {
    localStorage.setItem(REVIEW_KEY, next ? 'on' : 'off');
  } catch {
    /* storage unavailable: the choice lasts for this page */
  }
  listeners.forEach((f) => f());
}

const subscribe = (f: () => void) => { listeners.add(f); return () => { listeners.delete(f); }; };

/** This browser's choice, kept in step across the page. */
export const useReviewOn = () => useSyncExternalStore(subscribe, () => on, () => on);

let server: Promise<ReviewSettings> | null = null;

/** arenad's settings, asked once per page (a failure counts as off, and is asked again next time). */
function serverSettings(): Promise<ReviewSettings> {
  server ??= reviewSettings().catch((e) => {
    server = null;
    return { enabled: false, model: '', why: String(e?.message ?? e) };
  });
  return server;
}

/** arenad's settings: null while asking. */
export function useServerReview(): ReviewSettings | null {
  const [s, setS] = useState<ReviewSettings | null>(null);
  useEffect(() => {
    let live = true;
    serverSettings().then((x) => { if (live) setS(x); });
    return () => { live = false; };
  }, []);
  return s;
}

/** Whether review cards show: on in this browser and in arenad. */
export function useReviewEnabled(): boolean {
  const mine = useReviewOn();
  const s = useServerReview();
  return mine && !!s?.enabled;
}
