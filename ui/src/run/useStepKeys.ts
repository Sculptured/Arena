import { useEffect } from 'react';
import { hasModifier, isTyping, moveIn, navOf } from '@/lib/keys';

/** ↑/↓, ←/→, j/k and h/l move the selection through the steps in `order`, unless a field, dialog or handle has the keys. */
export function useStepKeys(order: number[], setSelected: (f: (s: number | null) => number | null) => void) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isTyping(e.target) || hasModifier(e) || (e.target as HTMLElement).closest?.('[role="dialog"],[role="separator"],[role="menu"]')) return;
      const nav = navOf(e.key, true);
      if (!nav || !order.length) return;
      e.preventDefault();
      setSelected((s) => moveIn(order, s, nav));
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [order, setSelected]);
}
