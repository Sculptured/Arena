import * as PopoverPrimitive from '@radix-ui/react-popover';
import type { ComponentProps } from 'react';
import { cn } from '@/lib/cn';

export const Popover = PopoverPrimitive.Root;
export const PopoverTrigger = PopoverPrimitive.Trigger;

export function PopoverContent({ className, align = 'start', ...props }: ComponentProps<typeof PopoverPrimitive.Content>) {
  return (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Content align={align} sideOffset={4}
        className={cn('z-50 max-h-[70vh] w-72 overflow-auto rounded-lg border border-line bg-raised p-2 text-sm shadow-pop', className)} {...props} />
    </PopoverPrimitive.Portal>
  );
}
