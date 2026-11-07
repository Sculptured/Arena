import { AlertTriangle, Inbox } from 'lucide-react';
import type { ReactNode } from 'react';
import { cn } from '@/lib/cn';
import { Button } from './button';

/** Nothing here yet, and what to do next. */
export function EmptyState({ title, hint, action, className }: { title: string; hint?: ReactNode; action?: ReactNode; className?: string }) {
  return (
    <div className={cn('flex flex-col items-center gap-2 px-4 py-10 text-center', className)}>
      <Inbox aria-hidden="true" className="size-5 text-faint" />
      <p className="text-sm font-medium">{title}</p>
      {hint && <p className="max-w-md text-xs text-muted">{hint}</p>}
      {action}
    </div>
  );
}

/** A failed load: what went wrong, what to try, and a retry when there is one. */
export function ErrorState({ error, hint, onRetry, className }: { error: string; hint?: ReactNode; onRetry?: () => void; className?: string }) {
  return (
    <div role="alert" className={cn('flex flex-wrap items-start gap-2 rounded-md bg-alarm-soft px-3 py-2 text-sm text-alarm', className)}>
      <AlertTriangle aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
      <div className="min-w-0 flex-1">
        <p className="font-medium break-words">{error}</p>
        <p className="text-xs text-fg/80">{hint ?? 'Check that arenad is running, then try again.'}</p>
      </div>
      {onRetry && <Button size="sm" onClick={onRetry}>Try again</Button>}
    </div>
  );
}

/** A neutral note: a fork that moved, a replayed part. */
export function Notice({ children, onDismiss, className }: { children: ReactNode; onDismiss?: () => void; className?: string }) {
  return (
    <div role="status" className={cn('flex flex-wrap items-center gap-2 rounded-md border border-line bg-raised px-3 py-1.5 text-sm', className)}>
      <span className="min-w-0 flex-1">{children}</span>
      {onDismiss && <Button size="sm" variant="ghost" onClick={onDismiss}>Dismiss</Button>}
    </div>
  );
}

/** Placeholder rows while something loads, and what is loading. */
export function LoadingState({ label, rows = 3, className }: { label: string; rows?: number; className?: string }) {
  return (
    <div className={cn('flex flex-col gap-2 p-3', className)} aria-busy="true">
      <span className="text-xs text-muted">{label}</span>
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="h-5 animate-pulse rounded-sm bg-sunken" style={{ width: `${90 - i * 12}%` }} />
      ))}
    </div>
  );
}
