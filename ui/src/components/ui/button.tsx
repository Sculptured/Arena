import { Slot } from '@radix-ui/react-slot';
import { type VariantProps, cva } from 'class-variance-authority';
import type { ComponentProps } from 'react';
import { cn } from '@/lib/cn';

export const buttonVariants = cva(
  'inline-flex shrink-0 items-center justify-center gap-1.5 whitespace-nowrap rounded-md border font-medium transition-colors disabled:pointer-events-none disabled:opacity-50 [&_svg]:size-3.5 [&_svg]:shrink-0',
  {
    variants: {
      variant: {
        default: 'border-line bg-raised text-fg shadow-panel hover:bg-hover',
        primary: 'border-primary bg-primary text-primary-fg hover:bg-primary-hover',
        ghost: 'border-transparent bg-transparent text-muted hover:bg-hover hover:text-fg',
        danger: 'border-alarm bg-raised text-alarm hover:bg-alarm-soft',
      },
      size: {
        sm: 'h-6 px-2 text-xs',
        md: 'h-7 px-2.5 text-sm',
        icon: 'size-7',
      },
    },
    defaultVariants: { variant: 'default', size: 'md' },
  },
);

export type ButtonProps = ComponentProps<'button'> & VariantProps<typeof buttonVariants> & { asChild?: boolean };

/** A button; `asChild` styles a link (or any element) as one. */
export function Button({ className, variant, size, asChild = false, type, ...props }: ButtonProps) {
  const Comp = asChild ? Slot : 'button';
  return <Comp className={cn(buttonVariants({ variant, size }), className)} type={asChild ? undefined : (type ?? 'button')} {...props} />;
}
