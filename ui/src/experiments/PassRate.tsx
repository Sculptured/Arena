import { Tooltip } from '../components/ui/tooltip';
import type { Column } from './model';

const pct = (x: number) => `${Math.round(x * 100)}%`;

/** The pass rate's 95% range as a faint band on a track, the rate itself as a mark on it. */
function RangeBar({ rate, range }: { rate: number; range: [number, number] }) {
  return (
    <Tooltip content={`95% range: ${pct(range[0])} to ${pct(range[1])}`}>
      <span className="relative block h-1.5 w-full rounded-full bg-sunken">
        <span className="absolute inset-y-0 rounded-full bg-line-strong/60" style={{ left: pct(range[0]), width: pct(range[1] - range[0]) }} />
        <span className="absolute top-1/2 size-2.5 -translate-1/2 rounded-full border-2 border-surface bg-fg" style={{ left: pct(rate) }} />
      </span>
    </Tooltip>
  );
}

/** "60%" large, "3 of 5 passed" under it, and the range it could really be. */
export default function PassRate({ c }: { c: Column }) {
  if (c.rate == null || !c.range) {
    return <div className="flex flex-col gap-2"><span className="text-2xl font-semibold text-faint">–</span><span className="text-xs text-muted">No run scored yet</span></div>;
  }
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-baseline gap-2">
        <span className="text-2xl font-semibold tracking-tight tabular-nums">{pct(c.rate)}</span>
        <span className="text-xs text-muted">{c.passed} of {c.scored} passed</span>
      </div>
      <RangeBar rate={c.rate} range={c.range} />
    </div>
  );
}
