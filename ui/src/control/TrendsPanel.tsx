import { useMemo } from 'react';
import { LineChart, Sparkline } from '../components/ui/chart';
import { EmptyState } from '../components/ui/state';
import type { Event } from '../types';
import type { Action } from './actions';
import { fmt } from './suite';
import { actionsPerBucket, bucketFor, cumulative, latencies, modelCalls, perMinute, secondsFrom } from './trends';

function Trend({ label, value, xs, ys }: { label: string; value: string; xs: number[]; ys: number[] }) {
  return (
    <div className="flex min-w-0 flex-col gap-1 rounded-md border border-line bg-raised px-2.5 py-2">
      <div className="flex items-baseline justify-between gap-2">
        <span className="truncate text-2xs font-medium text-muted">{label}</span>
        <span className="shrink-0 font-mono text-xs">{value}</span>
      </div>
      {ys.length > 1 ? <Sparkline xs={xs} ys={ys} label={`${label}: ${value}`} /> : <span className="h-7 text-2xs text-faint">{ys.length ? 'not enough points' : 'no line to draw'}</span>}
    </div>
  );
}

/** The series the panel draws, all from the run's own record. */
function useTrends(events: Event[], actions: Action[]) {
  return useMemo(() => {
    const bucketMs = bucketFor(actions);
    const per = actionsPerBucket(actions, bucketMs);
    const calls = modelCalls(events);
    const t0 = Math.min(...[...per.starts, ...calls.map((c) => c.at)].filter((t) => !Number.isNaN(t)));
    const total = cumulative(calls);
    const lat = latencies(actions);
    return { bucketMs, per, calls, t0, total, lat };
  }, [events, actions]);
}

/** Actions per minute per system, context per model call, tokens and cost as they build up, and tool latency. */
export default function TrendsPanel({ events, actions }: { events: Event[]; actions: Action[] }) {
  const t = useTrends(events, actions);
  if (!actions.length && !t.calls.length) return <EmptyState title="No trends yet" hint="Trends appear once the agents act on the world or call a model." />;
  const xsBuckets = secondsFrom(t.t0, t.per.starts);
  const xsCalls = secondsFrom(t.t0, t.calls.map((c) => c.at));
  const lastCost = t.total.cost[t.total.cost.length - 1];
  const peak = t.calls.length ? Math.max(...t.calls.map((c) => c.context)) : null;
  return (
    <div className="flex flex-col gap-3 p-3">
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 xl:grid-cols-4">
        {t.per.series.map((s) => (
          <Trend key={s.name} label={`${s.name} per min`} value={`${s.values.reduce((a, b) => a + b, 0)} actions`} xs={xsBuckets} ys={perMinute(s.values, t.bucketMs)} />
        ))}
        <Trend label="Tokens" value={fmt(t.total.tokens[t.total.tokens.length - 1] ?? null, 'tok')} xs={xsCalls} ys={t.total.tokens} />
        <Trend label="Cost" value={t.calls.length ? (lastCost == null ? 'not reported per call' : fmt(lastCost, 'usd')) : '–'} xs={xsCalls} ys={lastCost == null ? [] : t.total.cost.map((c) => c ?? 0)} />
        <Trend label="Tool latency" value={t.lat.length ? `max ${fmt(Math.max(...t.lat.map((l) => l.ms)) / 1000, 's')}` : '–'} xs={secondsFrom(t.t0, t.lat.map((l) => l.at))} ys={t.lat.map((l) => l.ms)} />
      </div>
      <div className="rounded-md border border-line bg-raised px-2.5 py-2">
        <div className="flex items-baseline justify-between"><span className="text-2xs font-medium text-muted">Context per model call (tokens)</span><span className="font-mono text-xs">peak {fmt(peak, 'tok')}</span></div>
        {t.calls.length > 1 ? <LineChart xs={xsCalls} ys={t.calls.map((c) => c.context)} label="Context per model call" /> : <p className="py-4 text-xs text-faint">No model call recorded its context size.</p>}
      </div>
      <p className="text-2xs text-faint">System lines are actions per minute over {t.bucketMs / 1000} s buckets. A call without a price leaves the cost line out; the run's cost is in the run page's header.</p>
    </div>
  );
}
