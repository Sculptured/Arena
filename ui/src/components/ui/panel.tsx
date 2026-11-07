import type { ComponentProps, ReactNode } from 'react';
import { cn } from '@/lib/cn';

/** A surface on the grey canvas: everything on a page sits in one. */
export function Panel({ className, ...props }: ComponentProps<'section'>) {
  return <section className={cn('min-w-0 rounded-lg border border-line bg-surface shadow-panel', className)} {...props} />;
}

/** A panel's title row: the title, an optional count or note, and actions on the right. */
export function PanelHeader({ title, meta, children, className }: { title: ReactNode; meta?: ReactNode; children?: ReactNode; className?: string }) {
  return (
    <header className={cn('flex min-h-9 flex-wrap items-center gap-x-3 gap-y-1 border-b border-line px-3 py-1.5', className)}>
      <h2 className="text-xs font-semibold text-muted">{title}</h2>
      {meta != null && <span className="text-xs text-faint">{meta}</span>}
      {children && <div className="ml-auto flex flex-wrap items-center gap-1.5">{children}</div>}
    </header>
  );
}

export function PanelBody({ className, ...props }: ComponentProps<'div'>) {
  return <div className={cn('p-3', className)} {...props} />;
}
