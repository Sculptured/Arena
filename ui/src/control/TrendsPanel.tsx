import { fmt } from './suite';
import { type Bucketed, type CallPoint, cumulative, sparkPoints } from './trends';

const W = 220;
const H = 36;

function Spark({ label, values, value }: { label: string; values: number[]; value: string }) {
  return (
    <div className="hmi-trend">
      <div className="small"><span className="label">{label}</span> <span className="mono">{value}</span></div>
      <svg width={W} height={H} viewBox={`0 0 ${W} ${H}`} aria-hidden="true">
        <polyline className="hmi-spark" points={sparkPoints(values, W, H)} />
      </svg>
    </div>
  );
}

/** Actions per minute per system, context per model call, and tokens and cost as they build up. */
export default function TrendsPanel({ perMinute, calls }: { perMinute: Bucketed; calls: CallPoint[] }) {
  const total = cumulative(calls);
  const lastCost = total.cost.length ? total.cost[total.cost.length - 1] : null;
  const peak = calls.length ? Math.max(...calls.map((c) => c.context)) : null;
  return (
    <div className="hmi-trends">
      {perMinute.series.map((s) => (
        <Spark key={s.name} label={`${s.name} actions / min`} values={s.values} value={`${s.values.reduce((a, b) => a + b, 0)} total`} />
      ))}
      <Spark label="Context per call" values={calls.map((c) => c.context)} value={`peak ${fmt(peak, 'tok')}`} />
      <Spark label="Tokens" values={total.tokens} value={fmt(total.tokens[total.tokens.length - 1] ?? null, 'tok')} />
      <div className="hmi-trend small"><span className="label">Cost</span> <span className="mono">{calls.length ? (lastCost == null ? 'unknown' : fmt(lastCost, 'usd')) : '–'}</span></div>
    </div>
  );
}
