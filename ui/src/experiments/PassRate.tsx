import { Tooltip } from '../components/ui/tooltip';
import type { Column } from './model';

const pct = (x: number) => `${Math.round(x * 100)}%`;

/**
 * The scored runs as one bar, passes in the result ink and failures in the alarm ink, and under it
 * the 95% range of the pass rate as a whisker on the same scale.
 */
function Bar({ c, rate, range }: { c: Column; rate: number; range: [number, number] }) {
  return (
    <Tooltip content={`${c.passed} of ${c.scored} passed\n95% range: ${pct(range[0])} to ${pct(range[1])}`}>
      <div className="flex flex-col gap-1.5" aria-label={`Pass rate ${pct(rate)}, 95% range ${pct(range[0])} to ${pct(range[1])}`}>
        <div className="flex h-2.5 w-full overflow-hidden rounded-full bg-line">
          <span className="h-full bg-ok" style={{ width: pct(rate) }} />
          <span className="h-full bg-alarm" style={{ width: pct(1 - rate) }} />
        </div>
        <div className="relative h-2" aria-hidden="true">
          <span className="absolute top-1/2 h-px -translate-y-1/2 bg-faint" style={{ left: pct(range[0]), width: pct(range[1] - range[0]) }} />
          <span className="absolute inset-y-0 w-px bg-faint" style={{ left: pct(range[0]) }} />
          <span className="absolute inset-y-0 w-px -translate-x-px bg-faint" style={{ left: pct(range[1]) }} />
        </div>
      </div>
    </Tooltip>
  );
}

/** The pass rate, the hero of a setup: the figure large, "3 of 5 passed", and the bar. */
export default function PassRate({ c }: { c: Column }) {
  if (c.rate == null || !c.range) {
    return (
      <div className="flex flex-col gap-3">
        <span className="text-3xl font-semibold tracking-tight text-faint">–</span>
        <div className="h-2.5 w-full rounded-full bg-line" />
        <span className="text-xs text-muted">{c.runs.some((r) => r.state === 'running') ? 'First run in progress' : 'No run scored yet'}</span>
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-baseline gap-2">
        <span className="text-3xl font-semibold tracking-tight tabular-nums">{pct(c.rate)}</span>
        <span className="text-sm text-muted">{c.passed} of {c.scored} passed</span>
      </div>
      <Bar c={c} rate={c.rate} range={c.range} />
    </div>
  );
}
