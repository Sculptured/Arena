import type { ComponentProps, ReactNode } from 'react';
import { cn } from '@/lib/cn';

/** A page's title row: the title and its state on the left, actions on the right, a meta line under. */
export function PageHeader({ title, status, meta, actions, children, mono = false }: {
  title: ReactNode; status?: ReactNode; meta?: ReactNode; actions?: ReactNode; children?: ReactNode; mono?: boolean;
}) {
  return (
    <header className="flex flex-col gap-2 border-b border-line bg-surface px-3 py-2.5 sm:px-4">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
        <h1 className={cn('min-w-0 truncate text-lg font-semibold', mono && 'font-mono text-base')}>{title}</h1>
        {status}
        {actions && <div className="ml-auto flex flex-wrap items-center gap-1.5">{actions}</div>}
      </div>
      {meta && <div className="flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-muted">{meta}</div>}
      {children}
    </header>
  );
}

/** The scrolling area under the header. */
export function PageBody({ className, ...props }: ComponentProps<'main'>) {
  return <main className={cn('flex min-w-0 flex-col gap-3 p-3 sm:p-4', className)} {...props} />;
}

/** A labelled figure in a header or an overview: the label small above, the value in mono below. */
export function Stat({ label, value, abnormal = false, title, children, className }: {
  label: string; value: ReactNode; abnormal?: boolean; title?: string; children?: ReactNode; className?: string;
}) {
  return (
    <div className={cn('flex min-w-0 flex-col', className)} title={title}>
      <span className="text-2xs font-medium whitespace-nowrap text-muted">{label}</span>
      <span className={cn('font-mono text-sm break-words', abnormal && 'font-semibold text-alarm')}>{value}</span>
      {children}
    </div>
  );
}
