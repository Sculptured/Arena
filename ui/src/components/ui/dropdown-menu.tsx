import * as Menu from '@radix-ui/react-dropdown-menu';
import type { ComponentProps } from 'react';
import { cn } from '@/lib/cn';

export const DropdownMenu = Menu.Root;
export const DropdownMenuTrigger = Menu.Trigger;

export function DropdownMenuContent({ className, align = 'end', ...props }: ComponentProps<typeof Menu.Content>) {
  return (
    <Menu.Portal>
      <Menu.Content align={align} sideOffset={4}
        className={cn('z-50 max-h-[60vh] min-w-48 overflow-auto rounded-lg border border-line bg-raised p-1 text-sm shadow-pop', className)} {...props} />
    </Menu.Portal>
  );
}

export function DropdownMenuItem({ className, ...props }: ComponentProps<typeof Menu.Item>) {
  return (
    <Menu.Item className={cn('flex cursor-default items-center gap-2 rounded-sm px-2 py-1 outline-none select-none data-[disabled]:opacity-50 data-[highlighted]:bg-hover', className)} {...props} />
  );
}

export function DropdownMenuLabel({ className, ...props }: ComponentProps<typeof Menu.Label>) {
  return <Menu.Label className={cn('px-2 py-1 text-2xs font-medium text-muted', className)} {...props} />;
}
