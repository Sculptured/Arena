import { ChevronUp } from 'lucide-react';
import { diskFill, diskLine, modesLine, useHealth } from '../components';
import { Popover, PopoverContent, PopoverTrigger } from '../components/ui/popover';
import { ProviderStatusLine } from '../ProviderFields';
import { bestProviders, checkProviders, providerSummary, useCheckedProviders } from '../providers';
import { cn } from '@/lib/cn';
import type { Health, ProviderStatus } from '../types';

/** API mode in one calm line; the full list (each provider, why, and Check) one click away. */
function ApiMode({ statuses }: { statuses: ProviderStatus[] }) {
  const checked = useCheckedProviders();
  const sum = providerSummary(statuses);
  if (!sum) return null;
  return (
    <Popover>
      <PopoverTrigger className="inline-flex min-w-0 items-center gap-1 rounded-sm px-1 hover:bg-hover hover:text-fg" title="Model providers for API-mode agents">
        <span className="text-faint">API mode</span>
        <span aria-hidden="true" className="font-mono">{sum.glyph}</span>
        <span className="truncate">{sum.text}</span>
        <ChevronUp aria-hidden="true" className="size-3 shrink-0" />
      </PopoverTrigger>
      <PopoverContent side="top" className="w-[min(420px,calc(100vw-24px))]">
        <p className="mb-2 text-2xs font-medium text-muted">API-mode providers</p>
        <ProviderStatusLine statuses={statuses} loading={checked.loading} error={checked.error} onCheck={() => { checkProviders().catch(() => {}); }} />
      </PopoverContent>
    </Popover>
  );
}

/** Saves used against the limit, as a small analog bar; abnormal only when new runs are refused. */
function Disk({ h }: { h: Health }) {
  const fill = diskFill(h);
  if (!fill) return null;
  return (
    <span className={cn('inline-flex items-center gap-1.5', fill.full && 'font-medium text-alarm')} title="Space used by saved worlds, and the limit above which new runs are refused">
      {fill.full && <span aria-hidden="true">⚠</span>}
      {diskLine(h)}{fill.full && ' (full: new runs refused)'}
      <span aria-hidden="true" className="relative h-1.5 w-14 overflow-hidden rounded-full bg-sunken ring-1 ring-line">
        <span className={cn('absolute inset-y-0 left-0', fill.full ? 'bg-alarm' : 'bg-line-strong')} style={{ width: `${fill.fraction * 100}%` }} />
      </span>
    </span>
  );
}

/** Saves this full or fuller are worth a look before the limit refuses new runs. */
const NEARLY_FULL = 0.9;

/** Only what needs attention, along the bottom: no model to run agents with, or saves nearly full. Nothing otherwise. */
export default function StatusBar() {
  const h = useHealth();
  const checked = useCheckedProviders();
  if (!h) return null;
  const statuses = bestProviders(checked.list, h.providers);
  const noCli = !h.claude && !h.codex;
  const noModel = noCli && !statuses.some((s) => s.available);
  const fill = diskFill(h);
  const disk = !!fill && fill.fraction >= NEARLY_FULL;
  if (!noModel && !disk) return null;
  return (
    // `status-line`: the hook the capture scripts (examples/*/capture) hide the bar by.
    <footer className="status-line bottom-0 z-40 flex min-h-7 flex-wrap items-center gap-x-4 gap-y-0.5 border-t border-line bg-surface px-3 py-0.5 text-xs text-muted sm:sticky">
      {noModel && <span className="font-medium text-alarm">⚠ No model to run agents with: {modesLine(h)}, and no API key is set.</span>}
      {noModel && <ApiMode statuses={statuses} />}
      {disk && <span className="ml-auto"><Disk h={h} /></span>}
    </footer>
  );
}
