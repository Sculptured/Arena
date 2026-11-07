// uPlot options for the two kinds of trend: a bare sparkline and a small chart with axes.
import type uPlot from 'uplot';

export interface ChartColors {
  series: string;
  axis: string;
  grid: string;
  /** Points drawn in the alarm colour (refusals, errors). */
  alarm: string;
}

/** A sparkline: one line, no axes, no legend, no cursor. */
export function sparkOptions(width: number, height: number, c: ChartColors): uPlot.Options {
  return {
    width, height, legend: { show: false }, cursor: { show: false }, select: { show: false, left: 0, top: 0, width: 0, height: 0 },
    scales: { x: { time: false }, y: { range: (_u, _min, max) => [0, Math.max(1, max)] } },
    axes: [{ show: false }, { show: false }],
    series: [{}, { stroke: c.series, width: 1.25, points: { show: false } }],
  };
}

/** A small chart: a time axis in seconds from the start, a value axis, one series. */
export function lineOptions(width: number, height: number, c: ChartColors, label: string): uPlot.Options {
  const axis = { stroke: c.axis, grid: { stroke: c.grid, width: 1 }, ticks: { stroke: c.grid, width: 1 }, font: '11px Inter Variable, system-ui' };
  return {
    width, height, legend: { show: false }, cursor: { y: false, points: { size: 5 } },
    scales: { x: { time: false } },
    axes: [{ ...axis, values: (_u, v) => v.map((s) => `${Math.round(s)} s`) }, { ...axis, size: 44 }],
    series: [{ label: 'time' }, { label, stroke: c.series, width: 1.25, points: { show: false } }],
  };
}

/** The chart colours from the current theme's tokens. */
export function themeColors(el: Element = document.documentElement): ChartColors {
  const css = getComputedStyle(el);
  const v = (n: string) => css.getPropertyValue(n).trim();
  return { series: v('--series'), axis: v('--muted'), grid: v('--line'), alarm: v('--alarm') };
}
