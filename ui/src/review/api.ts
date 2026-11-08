import { ApiError, call } from '../api';
import type { CompareReview, ReviewSettings, RunReview, StoredReview } from './types';

const id = encodeURIComponent;

/** What a review is of: one branch, two branches, or two suites. */
export type Target =
  | { kind: 'run'; branch: string }
  | { kind: 'compare'; a: string; b: string }
  | { kind: 'suites'; a: string; b: string };

/** The route of a target's review. */
export function reviewPath(t: Target): string {
  switch (t.kind) {
    case 'run': return `/api/branches/${id(t.branch)}/review`;
    case 'compare': return `/api/compare/review?a=${id(t.a)}&b=${id(t.b)}`;
    case 'suites': return `/api/suites/compare/review?a=${id(t.a)}&b=${id(t.b)}`;
  }
}

const withAgain = (path: string) => `${path}${path.includes('?') ? '&' : '?'}again=1`;

export const reviewSettings = () => call<ReviewSettings>('/api/review/settings');

/** The stored review, or null when there is none yet. */
export async function getReview<R extends RunReview | CompareReview>(t: Target): Promise<StoredReview<R> | null> {
  try {
    return await call<StoredReview<R>>(reviewPath(t));
  } catch (e) {
    if (e instanceof ApiError && e.status === 404) return null;
    throw e;
  }
}

/** Starts a review (or gets the stored one); `again` asks for a new one. */
export const startReview = <R extends RunReview | CompareReview>(t: Target, again = false) =>
  call<StoredReview<R>>(again ? withAgain(reviewPath(t)) : reviewPath(t), { method: 'POST' });
