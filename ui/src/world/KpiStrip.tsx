import { cn } from '@/lib/cn';
import type { TwinKpi } from './types';

/** The twin's KPIs in a strip: big mono figures, alarm colour only when abnormal, and the value before when it just changed. */
export default function KpiStrip({ kpis, before, clock }: { kpis: TwinKpi[]; before: Map<string, string>; clock?: string }) {
  return (
    <div className="flex flex-wrap gap-x-10 gap-y-3 px-1">
      {kpis.map((k) => {
        const was = before.get(k.id);
        return (
          <div key={k.id} className="flex min-w-0 flex-col">
            <span className="truncate text-2xs font-medium text-muted">{k.label}</span>
            <span className="flex min-w-0 items-baseline gap-2">
              <span className={cn('font-mono text-lg font-semibold whitespace-nowrap', k.abnormal && 'text-alarm', was != null && !k.abnormal && 'text-accent')}>{k.abnormal ? '⚠ ' : ''}{k.value}</span>
              {was != null && <span className="truncate text-xs font-medium text-accent">was {was}</span>}
            </span>
          </div>
        );
      })}
      {clock && (
        <div className="flex min-w-0 flex-col" title={clock}>
          <span className="text-2xs font-medium text-muted">World clock</span>
          <span className="font-mono text-lg font-semibold">{clock.split(' ')[1] ?? clock}</span>
        </div>
      )}
    </div>
  );
}
