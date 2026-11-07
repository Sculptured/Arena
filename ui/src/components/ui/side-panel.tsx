import { X } from 'lucide-react';
import type { ReactNode } from 'react';
import { cn } from '@/lib/cn';

/** A slim panel docked beside the main area (a step, an agent's prompt), with a title and a close button. */
export function SidePanel({ title, meta, onClose, children, className }: {
  title: ReactNode; meta?: ReactNode; onClose: () => void; children: ReactNode; className?: string;
}) {
  return (
    <aside className={cn('flex min-h-0 min-w-0 flex-col border-line bg-surface', className)} aria-label={typeof title === 'string' ? title : undefined}>
      <header className="flex items-start gap-2 border-b border-line px-3 py-2">
        <div className="flex min-w-0 flex-1 flex-col">
          <h2 className="text-sm font-semibold">{title}</h2>
          {meta && <span className="text-xs text-muted">{meta}</span>}
        </div>
        <button type="button" onClick={onClose} className="rounded-sm p-1 text-muted hover:bg-hover hover:text-fg" aria-label="Close"><X className="size-4" /></button>
      </header>
      <div className="min-h-0 flex-1 overflow-auto">{children}</div>
    </aside>
  );
}
