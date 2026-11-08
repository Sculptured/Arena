import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import type { BranchDetail, BranchStatus as Status, Change } from '../types';
import ReviewCard from './ReviewCard';

/** True once the branch was seen running here: its end is then reviewed at once. */
export function useSawLive(status: Status | undefined): boolean {
  const [saw, setSaw] = useState(status === 'running');
  useEffect(() => { if (status === 'running') setSaw(true); }, [status]);
  return saw;
}

/** What the run page reads from its address's state: the fork panel's pre-filled changes. */
export interface TryState { prefill?: Record<string, Change> }

/**
 * The review of a run on the Run screen: a run watched to its end is reviewed at once; "Rewind
 * and try this" opens the run page's fork panel at the step, the change pre-filled.
 */
export function LiveRunReview({ runId, d, onStep }: { runId: string; d: BranchDetail; onStep: (seq: number) => void }) {
  const nav = useNavigate();
  const auto = useSawLive(d.status);
  const onTry = (seq: number, prefill: Record<string, Change>) =>
    nav(`/runs/${encodeURIComponent(runId)}/${encodeURIComponent(d.id)}?step=${seq}&fork=1`, { state: { prefill } satisfies TryState });
  return <ReviewCard d={d} auto={auto} onStep={onStep} onTry={onTry} />;
}

/** The review of a run on its page; a run followed live here is reviewed when it ends. */
export function RunPageReview({ d, onStep, onTry }: { d: BranchDetail; onStep: (seq: number) => void; onTry: (seq: number, prefill: Record<string, Change>) => void }) {
  const auto = useSawLive(d.status);
  return <ReviewCard d={d} auto={auto} onStep={onStep} onTry={onTry} />;
}
