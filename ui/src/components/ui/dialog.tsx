import * as DialogPrimitive from '@radix-ui/react-dialog';
import { X } from 'lucide-react';
import type { ComponentProps, ReactNode } from 'react';
import { cn } from '@/lib/cn';
import { Button } from './button';

export const Dialog = DialogPrimitive.Root;
export const DialogTrigger = DialogPrimitive.Trigger;
export const DialogClose = DialogPrimitive.Close;

/** A centred dialog with a title, an optional description, and a close button. */
export function DialogContent({ title, description, className, children, bare = false, ...props }:
  ComponentProps<typeof DialogPrimitive.Content> & { title: string; description?: ReactNode; bare?: boolean }) {
  return (
    <DialogPrimitive.Portal>
      <DialogPrimitive.Overlay className="fixed inset-0 z-40 bg-black/30" />
      <DialogPrimitive.Content
        className={cn('fixed top-[12vh] left-1/2 z-50 flex max-h-[80vh] w-[min(560px,calc(100vw-24px))] -translate-x-1/2 flex-col overflow-hidden rounded-lg border border-line bg-surface shadow-pop', className)}
        {...props}>
        <div className={cn('border-b border-line py-3 pr-12 pl-4', bare && 'sr-only')}>
          <DialogPrimitive.Title className="text-base font-semibold">{title}</DialogPrimitive.Title>
          {description
            ? <DialogPrimitive.Description className="text-xs text-muted">{description}</DialogPrimitive.Description>
            : <DialogPrimitive.Description className="sr-only">{title}</DialogPrimitive.Description>}
        </div>
        {children}
        {!bare && <DialogPrimitive.Close className="absolute top-2.5 right-3 rounded-sm p-1 text-muted hover:bg-hover hover:text-fg" aria-label="Close"><X className="size-4" /></DialogPrimitive.Close>}
      </DialogPrimitive.Content>
    </DialogPrimitive.Portal>
  );
}

/** A dialog's footer: Cancel, and the primary action (`danger` for a destructive one). */
export function DialogActions({ onCancel, onGo, disabled, label, danger = false }: { onCancel: () => void; onGo: () => void; disabled?: boolean; label: string; danger?: boolean }) {
  return (
    <div className="flex justify-end gap-2 border-t border-line px-4 py-2.5">
      <Button onClick={onCancel}>Cancel</Button>
      <Button variant={danger ? 'danger' : 'primary'} onClick={onGo} disabled={disabled}>{label}</Button>
    </div>
  );
}
