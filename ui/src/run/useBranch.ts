import { useCallback, useEffect, useRef, useState } from 'react';
import { branch as fetchBranch, runs as fetchRuns, stream } from '../api';
import type { Branch, BranchDetail, Event } from '../types';

/** Adds a streamed event to the branch on screen, once. */
export function withEvent(d: BranchDetail | null, id: string, e: Event): BranchDetail | null {
  if (!d || d.id !== id || d.events.some((x) => x.seq === e.seq)) return d;
  return { ...d, events: [...d.events, e] };
}

/** A branch with its events, live while it runs; with `runId`, its run's other branches too. */
export function useBranch(branchId: string, runId?: string) {
  const [detail, setDetail] = useState<BranchDetail | null>(null);
  const [siblings, setSiblings] = useState<Branch[]>([]);
  const [error, setError] = useState<string | null>(null);
  const current = useRef(branchId);
  // A response only lands if it is still for the branch on screen.
  const load = useCallback(() => {
    const id = branchId;
    current.current = id;
    fetchBranch(id).then((d) => { if (current.current === id) { setDetail(d); setError(null); } })
      .catch((e) => { if (current.current === id) setError(String(e.message ?? e)); });
    if (runId) fetchRuns().then((r) => { if (current.current === id) setSiblings(r.find((x) => x.id === runId)?.branches ?? []); }).catch(() => {});
  }, [branchId, runId]);
  useEffect(() => { setDetail(null); load(); }, [load]);
  const running = detail?.status === 'running';
  useEffect(() => {
    if (!running) return;
    return stream(branchId, (e) => setDetail((d) => withEvent(d, branchId, e)), () => load());
  }, [running, branchId, load]);
  return { detail, siblings, error, reload: load };
}
