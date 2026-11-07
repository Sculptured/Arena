import { useEffect, useState } from 'react';
import { ApiError, restorePlan } from '../api';
import { REPLAY_NOTE, planLine } from '../restore';
import type { PartPlan, RestoreMode } from '../types';

/** What the restore plan for the panel's step says: loading, the parts, a refusal (409) or a failure. */
export type Plan =
  | { state: 'loading' }
  | { state: 'ready'; parts: PartPlan[] }
  | { state: 'refused'; reason: string }
  | { state: 'failed'; error: string };

const MODES: { id: RestoreMode; label: string; hint: string }[] = [
  { id: 'exact', label: 'Exact', hint: 'the world exactly as it was' },
  { id: 'approximate', label: 'Approximate', hint: 'from saves that may differ slightly' },
];

export const WAIT_REASON = 'Wait for this run to finish before forking.';

/** The plan for a fork of `branchId` at `seq` restored in `mode`. */
export async function loadPlan(branchId: string, seq: number, mode: RestoreMode): Promise<Plan> {
  try {
    return { state: 'ready', parts: (await restorePlan(branchId, seq, mode)).parts };
  } catch (e) {
    if (e instanceof ApiError && e.status === 409) return { state: 'refused', reason: e.message };
    return { state: 'failed', error: e instanceof Error ? e.message : String(e) };
  }
}

/** The plan for the current mode, reloaded when it changes. */
export function useRestorePlan(branchId: string, seq: number, mode: RestoreMode): Plan {
  const [plan, setPlan] = useState<Plan>({ state: 'loading' });
  useEffect(() => {
    let alive = true;
    setPlan({ state: 'loading' });
    loadPlan(branchId, seq, mode).then((p) => alive && setPlan(p));
    return () => { alive = false; };
  }, [branchId, seq, mode]);
  return plan;
}

/** Why the rerun can't start: the branch is running, or the restore is refused; null when it can. */
export function blockedReason(plan: Plan, running: boolean): string | null {
  if (running) return WAIT_REASON;
  return plan.state === 'refused' ? plan.reason : null;
}

function PlanLines({ plan }: { plan: Plan }) {
  if (plan.state === 'loading') return <span className="text-xs text-muted">Checking how each part is restored…</span>;
  // A refusal is said next to the start button it disables.
  if (plan.state === 'refused') return null;
  if (plan.state === 'failed') return <span className="text-xs text-muted">Couldn't get the restore plan: {plan.error}</span>;
  return (
    <>
      <ul className="flex flex-col gap-0.5">{plan.parts.map((p) => <li key={p.part} className="font-mono text-2xs">{planLine(p)}</li>)}</ul>
      {plan.parts.some((p) => p.how === 'replay') && <span className="text-xs text-muted">{REPLAY_NOTE}</span>}
    </>
  );
}

/** The "Restore" choice (Exact or Approximate) and, per part, how it would be restored. */
export function RestoreChoice({ mode, onMode, plan }: { mode: RestoreMode; onMode: (m: RestoreMode) => void; plan: Plan }) {
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex flex-wrap items-center gap-3 text-sm" role="radiogroup" aria-label="Restore">
        <span className="text-2xs font-medium text-muted">Restore</span>
        {MODES.map((m) => (
          <label key={m.id} className="flex items-center gap-1.5" title={m.hint}>
            <input type="radio" name="restore" value={m.id} checked={mode === m.id} onChange={() => onMode(m.id)} className="accent-[var(--accent)]" />
            {m.label}
          </label>
        ))}
      </div>
      <PlanLines plan={plan} />
    </div>
  );
}
