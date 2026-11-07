import { cn } from '@/lib/cn';
import type { TwinKpi } from './types';

/** The twin's KPIs in a strip: big mono figures, alarm colour only when abnormal, and the value before when it just changed. */
export default function KpiStrip({ kpis, before, clock }: { kpis: TwinKpi[]; before: Map<string, string>; clock?: string }) {
  return (
    <div className="grid grid-cols-2 gap-2 sm:grid-cols-[repeat(auto-fit,minmax(170px,1fr))]">
      {kpis.map((k) => {
        const was = before.get(k.id);
        return (
          <div key={k.id} className={cn('flex min-w-0 flex-col rounded-lg border bg-surface px-3 py-2 shadow-panel', k.abnormal ? 'border-alarm border-l-[3px]' : 'border-line', was != null && 'ring-2 ring-accent')}>
            <span className="truncate text-2xs font-medium text-muted">{k.label}</span>
            <span className="flex min-w-0 items-baseline gap-2">
              <span className={cn('font-mono text-xl font-semibold whitespace-nowrap', k.abnormal && 'text-alarm')}>{k.abnormal ? '⚠ ' : ''}{k.value}</span>
              {was != null && <span className="truncate text-xs font-medium text-accent">was {was}</span>}
            </span>
          </div>
        );
      })}
      {clock && (
        <div className="flex min-w-0 flex-col rounded-lg border border-line bg-surface px-3 py-2 shadow-panel">
          <span className="text-2xs font-medium text-muted">World clock</span>
          <span className="flex items-baseline gap-2"><span className="font-mono text-xl font-semibold">{clock.split(' ')[1] ?? clock}</span><span className="truncate font-mono text-xs text-faint">{clock.split(' ')[0]}</span></span>
        </div>
      )}
    </div>
  );
}
