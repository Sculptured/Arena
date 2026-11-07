import { Toaster as Sonner } from 'sonner';

/** Toasts in the token colours, bottom right. */
export function Toaster() {
  return (
    <Sonner position="bottom-right" toastOptions={{
      classNames: { toast: '!rounded-lg !border !border-line !bg-raised !text-fg !shadow-pop !text-sm', description: '!text-muted' },
    }} />
  );
}
