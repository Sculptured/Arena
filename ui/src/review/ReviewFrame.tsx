import { RotateCcw, Sparkles } from 'lucide-react';
import type { ReactNode } from 'react';
import { Button } from '../components/ui/button';
import { Panel, PanelHeader } from '../components/ui/panel';
import { ErrorState } from '../components/ui/state';
import type { CompareReview, RunReview, StoredReview } from './types';

/** "Reviewing…": a calm line while the model reads the record. */
export function Reviewing({ what }: { what: string }) {
  return (
    <div className="flex items-center gap-3 px-3 py-3" aria-busy="true">
      <span className="relative block h-1 w-24 shrink-0 overflow-hidden rounded-full bg-sunken"><span className="absolute inset-y-0 w-1/3 animate-slide rounded-full bg-line-strong" /></span>
      <span className="text-sm text-muted">Reviewing… A model is reading {what}. This takes a minute or so.</span>
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
  if (s.status === 'error') return <ErrorState className="m-3" error={s.error ?? 'The review failed.'} hint={retryHint} onRetry={onAgain} />;
  return s.review ? <>{done(s.review)}</> : null;
}

/**
 * A review card: its title, "Review again" once it has an answer, and the review in its state;
 * `empty` shows before there is any review, `footer` under a finished one.
 */
export function ReviewFrame<R extends RunReview | CompareReview>({ title, meta, label, h, what, retryHint, empty, done, footer }: {
  title: string; meta?: string; label: string; h: ReviewHandle<R>; what: string; retryHint: string;
  empty?: ReactNode; done: (r: R) => ReactNode; footer: (s: StoredReview<R>) => ReactNode;
}) {
  const { stored, error, start } = h;
  const finished = stored != null && stored.status !== 'running';
  return (
    <Panel aria-label={label}>
      <PanelHeader title={<span className="flex items-center gap-1.5"><Sparkles className="size-3.5" />{title}</span>} meta={meta}>
        {finished && <Button size="sm" variant="ghost" onClick={() => start(true)} title="Ask the model again"><RotateCcw />Review again</Button>}
      </PanelHeader>
      {error && <ErrorState className="m-3" error={error} hint="The review could not be started." onRetry={() => start()} />}
      {!stored && !error && empty}
      {stored && <Stored s={stored} what={what} retryHint={retryHint} onAgain={() => start(true)} done={done} />}
      {finished && footer(stored)}
    </Panel>
  );
}
