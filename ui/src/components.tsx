import { useEffect, useState } from 'react';
import { health } from './api';
import { APPROXIMATE_LABEL, isApproximate, planLine } from './restore';
import type { BranchStatus, Health, RestoredPart } from './types';

const STATUS: Record<BranchStatus, { glyph: string; label: string }> = {
  running: { glyph: '◐', label: 'running' },
  passed: { glyph: '✓', label: 'passed' },
  failed: { glyph: '✕', label: 'failed' },
  infra_error: { glyph: '⚠', label: 'infra error' },
  interrupted: { glyph: '■', label: 'interrupted' },
};

/** Shape + word, then colour. */
export function StatusChip({ status }: { status: BranchStatus }) {
  const s = STATUS[status];
  return (
    <span className={`chip status-${status}`}>
      <span className="chip-glyph" aria-hidden="true">{s.glyph}</span>
      {s.label}
    </span>
  );
}

/** "Approximate restore" on a branch (or compare side) some part of which was restored approximately. */
export function ApproximateMark({ of }: { of: { restored?: RestoredPart[] | null } }) {
  if (!isApproximate(of)) return null;
  return <span className="chip approx" title={of.restored?.map(planLine).join('\n')}>{APPROXIMATE_LABEL}</span>;
}

/** Pages dispatch this after they change how much disk the arena uses (e.g. delete a run). */
export const DISK_CHANGED = 'arena:disk-changed';

export function useHealth(): Health | null {
  const [h, setH] = useState<Health | null>(null);
  useEffect(() => {
    let alive = true;
    const load = () => health().then((v) => alive && setH(v)).catch(() => {});
    load();
    window.addEventListener(DISK_CHANGED, load);
    return () => {
      alive = false;
      window.removeEventListener(DISK_CHANGED, load);
    };
  }, []);
  return h;
}

/** "Saves 0.4 / 5 GB" (every save the arena keeps; GB = 2^30 bytes); empty when the server doesn't report disk use. */
export function diskLine(h: Health | null): string {
  if (!h?.disk) return '';
  const gb = (n: number) => {
    const v = n / 2 ** 30;
    return v >= 10 ? Math.round(v).toString() : (Math.round(v * 10) / 10).toString();
  };
  return `Saves ${gb(h.disk.bytes)} / ${gb(h.disk.limit_bytes)} GB`;
}

export function modesLine(h: Health | null): string {
  if (!h) return '';
  return [
    h.claude && 'Claude Code',
    h.codex && 'Codex',
    h.open_available ? 'OpenRouter' : 'OpenRouter (needs key)',
  ].filter(Boolean).join(' · ');
}

export function formatTime(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
}
