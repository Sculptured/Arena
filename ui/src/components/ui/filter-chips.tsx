import { cn } from '@/lib/cn';

/** One-of filter chips, each with its count; a chip with nothing in it stays visible but quiet. */
export function FilterChips<K extends string>({ options, value, counts, onChange, label }: {
  options: { id: K; label: string }[]; value: K; counts?: Partial<Record<K, number>>; onChange: (k: K) => void; label: string;
}) {
  return (
    <div role="radiogroup" aria-label={label} className="flex flex-wrap items-center gap-1">
      {options.map((o) => {
        const n = counts?.[o.id];
        return (
          <button key={o.id} role="radio" aria-checked={value === o.id} onClick={() => onChange(o.id)}
            className={cn('inline-flex h-6 items-center gap-1.5 rounded-md border border-line px-2 text-xs text-muted hover:bg-hover hover:text-fg',
              value === o.id && 'border-line-strong bg-raised font-medium text-fg shadow-panel', n === 0 && 'opacity-60')}>
            {o.label}{n != null && <span className="font-mono text-2xs text-faint">{n}</span>}
          </button>
        );
      })}
    </div>
  );
}
