import { useEffect, useState } from 'react';
import { health } from './api';
import { StatusMark, Tag } from './components/ui/status';
import { runMark } from './control/suite';
import { APPROXIMATE_LABEL, isApproximate, planLine } from './restore';
import type { BranchStatus, Health, RestoredPart } from './types';

/** A branch's status: shape + word, then colour (only when abnormal). */
export function StatusChip({ status }: { status: BranchStatus }) {
  return <StatusMark mark={runMark(status)} />;
}

/** "Approximate restore" on a branch (or compare side) some part of which was restored approximately. */
export function ApproximateMark({ of }: { of: { restored?: RestoredPart[] | null } }) {
  if (!isApproximate(of)) return null;
  return <Tag tone="warn" title={of.restored?.map(planLine).join('\n')}>{APPROXIMATE_LABEL}</Tag>;
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

const gb = (n: number) => {
  const v = n / 2 ** 30;
  return v >= 10 ? Math.round(v).toString() : (Math.round(v * 10) / 10).toString();
};

/** "Saves 0.4 / 5 GB" (every save the arena keeps; GB = 2^30 bytes); empty when the server doesn't report disk use. */
export function diskLine(h: Health | null): string {
  if (!h?.disk) return '';
  return `Saves ${gb(h.disk.bytes)} / ${gb(h.disk.limit_bytes)} GB`;
}

/** How full the saves are, from 0 to 1, and whether new runs are refused; null without disk info. */
export function diskFill(h: Health | null): { fraction: number; full: boolean } | null {
  if (!h?.disk || h.disk.limit_bytes <= 0) return null;
  return { fraction: Math.min(1, h.disk.bytes / h.disk.limit_bytes), full: h.disk.bytes >= h.disk.limit_bytes };
}

/** The CLI modes found on this machine; API mode's providers have their own line. */
export function modesLine(h: Health | null): string {
  if (!h) return '';
  const found = [h.claude && 'Claude Code', h.codex && 'Codex'].filter(Boolean).join(' · ');
  return found || 'No agent CLI on PATH';
}

export function formatTime(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
}

/** "3 min ago", "2 h ago", "4 Nov": how long ago, for dense lists. */
export function ago(iso: string, now: number = Date.now()): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  const s = Math.max(0, (now - t) / 1000);
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return new Date(t).toLocaleDateString([], { day: 'numeric', month: 'short' });
}
