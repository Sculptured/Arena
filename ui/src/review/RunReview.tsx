import { useEffect, useState } from 'react';
import type { BranchDetail, BranchStatus as Status } from '../types';
import ReviewCard from './ReviewCard';

/** True once the branch was seen running here: its end is then reviewed at once. */
export function useSawLive(status: Status | undefined): boolean {
  const [saw, setSaw] = useState(status === 'running');
  useEffect(() => { if (status === 'running') setSaw(true); }, [status]);
  return saw;
}

/** The review of a run: a run watched to its end here is reviewed at once. */
export function RunReview({ d, onStep }: { d: BranchDetail; onStep: (seq: number) => void }) {
  const auto = useSawLive(d.status);
  return <ReviewCard d={d} auto={auto} onStep={onStep} />;
}
