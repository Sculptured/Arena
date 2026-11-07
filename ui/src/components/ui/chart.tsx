import { useCallback, useEffect, useMemo, useRef } from 'react';
import uPlot from 'uplot';
import { lineOptions, sparkOptions, themeColors } from '@/lib/chart';
import { useTheme } from '@/app/theme';

type Make = (width: number, colors: ReturnType<typeof themeColors>) => uPlot.Options;

/** Mounts a uPlot chart that fills its box's width and follows the theme. */
function usePlot(data: uPlot.AlignedData, make: Make) {
  const box = useRef<HTMLDivElement>(null);
  const theme = useTheme().resolved;
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const plot = new uPlot(make(Math.max(40, el.clientWidth), themeColors()), data, el);
    const ro = new ResizeObserver(() => plot.setSize({ width: Math.max(40, el.clientWidth), height: plot.height }));
    ro.observe(el);
    return () => { ro.disconnect(); plot.destroy(); };
  }, [data, make, theme]);
  return box;
}

/** A bare trend line: the shape of a series over a run. */
export function Sparkline({ xs, ys, height = 28, label }: { xs: number[]; ys: number[]; height?: number; label: string }) {
  const data = useMemo<uPlot.AlignedData>(() => [xs, ys], [xs, ys]);
  const make = useCallback<Make>((w, c) => sparkOptions(w, height, c), [height]);
  const box = usePlot(data, make);
  return <div ref={box} role="img" aria-label={label} className="w-full" style={{ height }} />;
}

/** A small chart with a time axis (seconds from the start) and a value axis. */
export function LineChart({ xs, ys, height = 140, label }: { xs: number[]; ys: number[]; height?: number; label: string }) {
  const data = useMemo<uPlot.AlignedData>(() => [xs, ys], [xs, ys]);
  const make = useCallback<Make>((w, c) => lineOptions(w, height, c, label), [height, label]);
  const box = usePlot(data, make);
  return <div ref={box} role="img" aria-label={label} className="w-full" style={{ height }} />;
}
