import type { ComponentProps } from 'react';
import { cn } from '@/lib/cn';
import type { Mark } from '@/lib/mark';

export type { Mark };

/** Glyph + word, then colour: abnormal states get the alarm colour and a tinted chip. */
export function StatusMark({ mark, className, title }: { mark: Mark; className?: string; title?: string }) {
  return (
    <span
      title={title}
      className={cn(
        'inline-flex shrink-0 items-center gap-1 rounded-sm px-1 text-xs whitespace-nowrap',
        mark.abnormal ? 'bg-alarm-soft font-medium text-alarm' : 'text-muted',
        className,
      )}
    >
      <span aria-hidden="true" className="w-3 text-center font-mono">{mark.glyph}</span>
      {mark.word}
    </span>
  );
}

/** A neutral label: "simulated", "safety", a system kind. `tone="warn"` for a caution worth noticing. */
export function Tag({ className, tone, ...props }: ComponentProps<'span'> & { tone?: 'warn' | 'alarm' }) {
  return (
    <span
      className={cn(
        'inline-flex shrink-0 items-center gap-1 rounded-sm border px-1 text-2xs leading-4 whitespace-nowrap',
        tone === 'warn' ? 'border-transparent bg-warn-soft text-warn' : tone === 'alarm' ? 'border-transparent bg-alarm-soft text-alarm' : 'border-line text-muted',
        className,
      )}
      {...props}
    />
  );
}
