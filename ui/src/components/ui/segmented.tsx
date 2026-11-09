import { cn } from '@/lib/cn';

/** A few mutually exclusive choices side by side, the chosen one raised. */
export function Segmented<T extends string | number>({ value, options, onChange, label, disabled, className }: {
  value: T; options: readonly { id: T; label: string }[]; onChange: (v: T) => void; label: string; disabled?: boolean; className?: string;
}) {
  return (
    <div role="radiogroup" aria-label={label} className={cn('inline-flex rounded-md bg-sunken p-0.5', className)}>
      {options.map((o) => (
        <button key={String(o.id)} type="button" role="radio" aria-checked={o.id === value} disabled={disabled} onClick={() => onChange(o.id)}
          className={cn('h-7 min-w-10 rounded-[3px] px-3 text-sm tabular-nums transition-colors disabled:opacity-50',
            o.id === value ? 'bg-raised font-medium text-fg shadow-panel ring-1 ring-line' : 'text-muted hover:text-fg')}>
          {o.label}
        </button>
      ))}
    </div>
  );
}
