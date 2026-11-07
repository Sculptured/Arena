import type { ComponentProps } from 'react';
import { cn } from '@/lib/cn';

/** A dense table that scrolls sideways inside its panel on narrow screens. */
export function Table({ className, wrapperClassName, ...props }: ComponentProps<'table'> & { wrapperClassName?: string }) {
  return (
    <div className={cn('min-w-0 overflow-x-auto', wrapperClassName)}>
      <table className={cn('w-full border-collapse text-sm', className)} {...props} />
    </div>
  );
}

export function Th({ className, ...props }: ComponentProps<'th'>) {
  return <th className={cn('h-8 border-b border-line bg-sunken px-3 text-left text-2xs font-medium whitespace-nowrap text-muted', className)} {...props} />;
}

/** A row header cell: the first column naming the row. */
export function RowTh({ className, ...props }: ComponentProps<'th'>) {
  return <th scope="row" className={cn('h-8 border-b border-line px-3 text-left font-normal whitespace-nowrap', className)} {...props} />;
}

export function Td({ className, ...props }: ComponentProps<'td'>) {
  return <td className={cn('h-8 border-b border-line px-3 whitespace-nowrap', className)} {...props} />;
}

export function Tr({ className, ...props }: ComponentProps<'tr'>) {
  return <tr className={cn('last:[&>*]:border-b-0', className)} {...props} />;
}
