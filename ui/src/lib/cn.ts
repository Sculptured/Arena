import { type ClassValue, clsx } from 'clsx';
import { twMerge } from 'tailwind-merge';

/** Class names joined, later Tailwind utilities winning over earlier ones. */
export const cn = (...values: ClassValue[]) => twMerge(clsx(values));
