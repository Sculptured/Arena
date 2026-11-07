import { useEffect, useLayoutEffect, useMemo, useState } from 'react';
import { views as fetchViews } from '../api';
import { hasModifier, isTyping, navOf } from '@/lib/keys';
import type { BranchDetail } from '../types';
import { moveStop, snapStop } from './scrub';
import { normalizeViews, viewsFromEvents } from './twin';
import type { BranchView } from './types';

/**
 * The branch's views: `GET /api/branches/{id}/views` (inherited views included), refetched as a
 * live run records steps; the views in its own events when the endpoint fails or is missing.
 */
export function useViews(branchId: string, detail: BranchDetail | null) {
  const [raw, setRaw] = useState<BranchView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const count = detail?.events.length ?? 0;
  useEffect(() => {
    let alive = true;
    fetchViews(branchId)
      .then((v) => { if (alive) { setRaw(v); setError(null); } })
      .catch((e) => { if (alive) { setRaw(null); setError(String(e.message ?? e)); } });
    return () => { alive = false; };
  }, [branchId, count]);
  const views = useMemo(() => {
    if (!detail) return [];
    return raw ? normalizeViews(raw) : viewsFromEvents(detail.events);
  }, [raw, detail]);
  return { views, error };
}

const PLAY_MS = 1300;

/** Play and pause through the stops, one every 1.3 s, stopping at the last. */
export function usePlay(stops: number[], step: number | null, setStep: (n: number | null) => void) {
  const [playing, setPlaying] = useState(false);
  const cur = snapStop(stops, step);
  useEffect(() => {
    if (!playing) return;
    const id = window.setTimeout(() => {
      const next = moveStop(stops, cur, 'next');
      if (next == null || next === cur) setPlaying(false);
      else setStep(next);
    }, PLAY_MS);
    return () => window.clearTimeout(id);
  }, [playing, cur, stops, setStep]);
  const toggle = () => {
    if (!playing && cur === stops.at(-1) && stops.length > 1) setStep(stops[0]);
    setPlaying((p) => !p);
  };
  return { playing, toggle, pause: () => setPlaying(false) };
}

/** ←/→ (and h/l, Home/End) move between stops and Space plays or pauses, unless a field, dialog or menu has the keys. */
export function useStopKeys(stops: number[], setStep: (f: (s: number | null) => number | null) => void, onPlay: () => void, onMove: () => void) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (isTyping(t) || hasModifier(e) || t.closest?.('[role="dialog"],[role="menu"],[role="separator"]')) return;
      if (e.key === ' ') {
        if (!t.closest?.('button,a')) { e.preventDefault(); onPlay(); }
        return;
      }
      const nav = navOf(e.key, true);
      if (!nav || !stops.length) return;
      e.preventDefault();
      onMove();
      setStep((s) => moveStop(stops, s, nav));
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [stops, setStep, onPlay, onMove]);
}

/** The width of an element, followed as it resizes. */
export function useWidth<T extends HTMLElement>() {
  const [el, setEl] = useState<T | null>(null);
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    if (!el) return;
    setWidth(Math.floor(el.clientWidth));
    const ro = new ResizeObserver(([entry]) => setWidth(Math.floor(entry.contentRect.width)));
    ro.observe(el);
    return () => ro.disconnect();
  }, [el]);
  return { ref: setEl, width };
}
