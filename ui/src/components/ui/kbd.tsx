import type { ComponentProps } from 'react';
import { cn } from '@/lib/cn';

export function Kbd({ className, ...props }: ComponentProps<'kbd'>) {
  return <kbd className={cn('inline-flex h-4.5 min-w-4.5 items-center justify-center rounded-sm border border-line bg-raised px-1 font-mono text-2xs text-muted', className)} {...props} />;
}
