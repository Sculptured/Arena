import type { ComponentProps, ReactNode } from 'react';
import { cn } from '@/lib/cn';

/** The page widths: a form, a list, results side by side, or the whole window (a timeline, a twin). */
export type Width = 'narrow' | 'default' | 'wide' | 'full';
const WIDTH: Record<Width, string> = { narrow: 'mx-auto w-full max-w-3xl', default: 'mx-auto w-full max-w-5xl', wide: 'mx-auto w-full max-w-7xl', full: '' };

/** A page's title row: the title and its state on the left, actions on the right, a meta line under. */
export function PageHeader({ title, status, meta, actions, children, width = 'full', className }: {
  title: ReactNode; status?: ReactNode; meta?: ReactNode; actions?: ReactNode; children?: ReactNode; width?: Width; className?: string;
}) {
  return (
    <header className={cn('flex flex-col gap-2 px-4 pt-6 pb-1 sm:px-6', WIDTH[width], className)}>
      <div className="flex min-h-8 flex-wrap items-center gap-x-3 gap-y-2">
        <h1 className="min-w-0 truncate text-xl font-semibold tracking-tight">{title}</h1>
        {status}
        {actions && <div className="ml-auto flex flex-wrap items-center gap-1.5">{actions}</div>}
      </div>
      {meta && <div className="flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-muted">{meta}</div>}
      {children}
    </header>
  );
}

/** The scrolling area under the header. */
export function PageBody({ className, width = 'full', ...props }: ComponentProps<'main'> & { width?: Width }) {
  return <main className={cn('flex min-w-0 flex-col gap-4 px-4 py-4 sm:px-6', WIDTH[width], className)} {...props} />;
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
