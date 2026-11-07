import type { CSSProperties } from 'react';
import { StatusMark } from '../components/ui/status';
import { cn } from '@/lib/cn';
import { isIdLike, itemFactParts, itemFacts, statusMark } from './twin';
import type { AreaKind, TwinItem } from './types';

/** A simple equipment shape per kind of area, in the process view's style. */
export function ZoneGlyph({ kind }: { kind: AreaKind }) {
  const c = { fill: 'var(--equip)', stroke: 'var(--equip-line)', strokeWidth: 1.25 };
  return (
    <svg width="18" height="18" viewBox="0 0 22 22" aria-hidden="true" className="shrink-0">
      {kind === 'plant' && <><rect x="2" y="9" width="18" height="10" {...c} /><path d="M5 9V4h3v5M11 9V6h3v3" {...c} /></>}
      {kind === 'lab' && <path d="M8 2h6M9 2v6l-6 11h16L13 8V2" {...c} />}
      {kind === 'warehouse' && <><path d="M2 9 11 3l9 6v11H2Z" {...c} /><path d="M6 20v-7h10v7" {...c} /></>}
      {kind === 'carriers' && <><rect x="1.5" y="6" width="12" height="9" {...c} /><path d="M13.5 9h4l3 3v3h-7Z" {...c} /><circle cx="6" cy="17" r="2" {...c} /><circle cx="16" cy="17" r="2" {...c} /></>}
      {kind === 'customers' && <><rect x="3" y="4" width="16" height="16" {...c} /><path d="M7 8h2M13 8h2M7 12h2M13 12h2M10 20v-4h2v4" {...c} /></>}
      {kind === 'approvals' && <path d="M11 2 20 11 11 20 2 11Z" {...c} />}
      {!['plant', 'lab', 'warehouse', 'carriers', 'customers', 'approvals'].includes(kind) && <rect x="3" y="4" width="16" height="14" rx="1" {...c} />}
    </svg>
  );
}

/**
 * One item: its name, its status as a glyph and a word (alarm colour only when abnormal), its
 * facts, and, when it changed since the view before, what changed.
 */
export function TwinTile({ item, change, style, className }: { item: TwinItem; change?: string; style?: CSSProperties; className?: string }) {
  const facts = itemFacts(item);
  const parts = itemFactParts(item);
  return (
    <div style={style} title={[item.label, item.status, facts, change && `changed: ${change}`].filter(Boolean).join('\n')}
      className={cn('flex min-w-0 flex-col justify-start gap-0 overflow-hidden rounded-md border bg-raised px-2 py-1 shadow-panel',
        item.abnormal ? 'border-alarm border-l-[3px]' : 'border-line-strong', change && 'ring-2 ring-accent', className)}>
      <span className={cn('truncate text-xs font-semibold', isIdLike(item.label) && 'font-mono')}>{item.label}</span>
      <span className="flex min-w-0 items-center gap-1"><StatusMark className="-ml-1 min-w-0" mark={statusMark(item)} /></span>
      {parts.length > 0 && (
        <span className="line-clamp-2 text-2xs leading-[14px] text-muted">
          {parts.map((p, i) => <span key={i}>{i > 0 && ' · '}<span className="whitespace-nowrap">{p}</span></span>)}
        </span>
      )}
      {change && <span className="-mx-2 mt-auto -mb-1 truncate bg-accent-soft px-2 py-px text-2xs font-medium text-accent">▸ {change}</span>}
    </div>
  );
}
