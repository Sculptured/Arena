import { ChevronDown, ChevronUp, RotateCcw, Sparkles } from 'lucide-react';
import { type ReactNode, useState } from 'react';
import { Button } from '../components/ui/button';
import { ErrorState } from '../components/ui/state';
import type { CompareReview, RunReview, StoredReview } from './types';

/** "Reviewing…": a calm line while the model reads the record. */
export function Reviewing({ what }: { what: string }) {
  return (
    <div className="flex items-center gap-3 py-1" aria-busy="true">
      <span className="relative block h-1 w-24 shrink-0 overflow-hidden rounded-full bg-sunken"><span className="absolute inset-y-0 w-1/3 animate-slide rounded-full bg-line-strong" /></span>
      <span className="text-sm text-muted" title={`A model is reading ${what}. This takes a minute or so.`}>Reviewing…</span>
    </div>
  );
}

/** What `useReview` gives a card. */
export interface ReviewHandle<R> {
  stored: StoredReview<R> | null;
  error: string | null;
  start: (again?: boolean) => void;
}

/** The stored review in its state: running, failed, or done (drawn by `done`). */
function Stored<R>({ s, what, retryHint, onAgain, done }: {
  s: StoredReview<R>; what: string; retryHint: string; onAgain: () => void; done: (r: R) => ReactNode;
}) {
  if (s.status === 'running') return <Reviewing what={what} />;
  if (s.status === 'error') return <ErrorState error={s.error ?? 'The review failed.'} hint={retryHint} onRetry={onAgain} />;
  return s.review ? <>{done(s.review)}</> : null;
}

/** A finished review on one line: its headline and More. */
function OneLine({ title, text, onMore }: { title?: string; text: string; onMore: () => void }) {
  return (
    <div className="flex items-start gap-2 text-sm">
      <Sparkles className="mt-0.5 size-3.5 shrink-0 text-muted" aria-hidden="true" />
      <p className="min-w-0 flex-1 leading-5">{title && <span className="font-semibold">{title}: </span>}{text}</p>
      <Button size="sm" variant="ghost" onClick={onMore}><ChevronDown />More</Button>
    </div>
  );
}

/** A finished review in full, with Review again and Less. */
function Full({ onAgain, onLess, children }: { onAgain: () => void; onLess: () => void; children: ReactNode }) {
  return (
    <div className="rounded-lg bg-surface">
      <div className="flex justify-end gap-1 px-2 pt-2">
        <Button size="sm" variant="ghost" onClick={onAgain} title="Ask the model again"><RotateCcw />Review again</Button>
        <Button size="sm" variant="ghost" onClick={onLess}><ChevronUp />Less</Button>
      </div>
      {children}
    </div>
  );
}

/**
 * A review: one line (its headline and More) until opened; open, the whole review with Less and
 * Review again. `empty` shows before there is any review, `footer` under an open one.
 */
export function ReviewFrame<R extends RunReview | CompareReview>({ title, label, h, what, retryHint, empty, done, headline, footer }: {
  title?: string; label: string; h: ReviewHandle<R>; what: string; retryHint: string;
  empty?: ReactNode; done: (r: R) => ReactNode; headline: (r: R) => string; footer: (s: StoredReview<R>) => ReactNode;
}) {
  const { stored, error, start } = h;
  const [open, setOpen] = useState(false);
  const review = stored?.status === 'done' ? stored.review : null;
  const body = (): ReactNode => {
    if (error) return <ErrorState error={error} hint="The review could not be started." onRetry={() => start()} />;
    if (!stored) return empty;
    if (!review) return <Stored s={stored} what={what} retryHint={retryHint} onAgain={() => start(true)} done={done} />;
    if (!open) return <OneLine title={title} text={headline(review)} onMore={() => setOpen(true)} />;
    return <Full onAgain={() => start(true)} onLess={() => setOpen(false)}>{done(review)}{footer(stored)}</Full>;
  };
  return <section aria-label={label} className="min-w-0">{body()}</section>;
}
