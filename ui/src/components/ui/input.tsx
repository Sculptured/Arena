import type { ComponentProps, ReactNode } from 'react';
import { cn } from '@/lib/cn';

const control = 'h-7 w-full min-w-0 rounded-md border border-line bg-raised px-2 text-sm text-fg placeholder:text-faint disabled:opacity-50';

export function Input({ className, ...props }: ComponentProps<'input'>) {
  return <input className={cn(control, className)} {...props} />;
}

export function Textarea({ className, ...props }: ComponentProps<'textarea'>) {
  return <textarea className={cn(control, 'h-auto py-1.5 font-mono text-xs leading-5', className)} {...props} />;
}

/** The platform select, styled: right on every device, and keyboard-complete. */
export function NativeSelect({ className, ...props }: ComponentProps<'select'>) {
  return <select className={cn(control, 'pr-6', className)} {...props} />;
}

export function Checkbox({ className, ...props }: Omit<ComponentProps<'input'>, 'type'>) {
  return <input type="checkbox" className={cn('size-3.5 shrink-0 accent-[var(--accent)]', className)} {...props} />;
}

/** A label above its control. */
export function Field({ label, hint, className, children }: { label: ReactNode; hint?: ReactNode; className?: string; children: ReactNode }) {
  return (
    <label className={cn('flex min-w-0 flex-col gap-1', className)}>
      <span className="text-2xs font-medium tracking-wide text-muted uppercase">{label}</span>
      {children}
      {hint && <span className="text-xs text-muted">{hint}</span>}
    </label>
  );
}
