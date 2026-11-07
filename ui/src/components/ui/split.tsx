import { type KeyboardEvent, type PointerEvent, type ReactNode, useRef, useState } from 'react';
import { type Limits, clampWidth, nudge, readWidth, storeWidth } from '@/lib/split';
import { cn } from '@/lib/cn';

/** The side panel's width and the handlers of the drag handle. */
function useSideWidth(storageKey: string, initial: number, limits: Limits) {
  const [width, setWidth] = useState(() => clampWidth(readWidth(storageKey, initial), limits));
  const root = useRef<HTMLDivElement>(null);
  const set = (w: number) => {
    const next = clampWidth(w, limits, root.current?.clientWidth ?? Infinity);
    setWidth(next);
    storeWidth(storageKey, next);
  };
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    const box = root.current?.getBoundingClientRect();
    if (!box) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    const move = (ev: globalThis.PointerEvent) => set(box.right - ev.clientX);
    const up = () => { window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up); };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  };
  const onKeyDown = (e: KeyboardEvent) => {
    const w = nudge(width, e.key, e.shiftKey);
    if (w == null) return;
    e.preventDefault();
    set(w);
  };
  return { width, root, onPointerDown, onKeyDown };
}

/** A main area and a side panel on the right that the reader can resize; stacked on narrow screens. */
export function Split({ main, side, storageKey, initial = 420, limits = { min: 300, max: 760 }, className }: {
  main: ReactNode; side: ReactNode; storageKey: string; initial?: number; limits?: Limits; className?: string;
}) {
  const s = useSideWidth(storageKey, initial, limits);
  return (
    <div ref={s.root} className={cn('flex min-h-0 flex-col gap-3 lg:flex-row lg:gap-0', className)} style={{ ['--side-w' as string]: `${s.width}px` }}>
      <div className="min-w-0 flex-1">{main}</div>
      <div role="separator" aria-orientation="vertical" aria-label="Resize the side panel" aria-valuenow={s.width}
        aria-valuemin={limits.min} aria-valuemax={limits.max} tabIndex={0}
        onPointerDown={s.onPointerDown} onKeyDown={s.onKeyDown}
        className="group hidden w-3 shrink-0 cursor-col-resize touch-none justify-center lg:flex">
        <span className="h-full w-px bg-line group-hover:w-0.5 group-hover:bg-accent group-focus-visible:w-0.5 group-focus-visible:bg-accent" />
      </div>
      <aside className="min-w-0 lg:w-[var(--side-w)] lg:shrink-0">{side}</aside>
    </div>
  );
}
