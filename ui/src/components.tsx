import { useEffect, useState } from 'react';
import { health } from './api';
import type { BranchStatus, Health } from './types';

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

export function useHealth(): Health | null {
  const [h, setH] = useState<Health | null>(null);
  useEffect(() => {
    let alive = true;
    health().then((v) => alive && setH(v)).catch(() => {});
    return () => {
      alive = false;
    };
  }, []);
  return h;
}

export function modesLine(h: Health | null): string {
  if (!h) return '';
  return [
    h.claude && 'Claude Code',
    h.codex && 'Codex',
    h.open_available ? 'Open' : 'Open (needs key)',
  ].filter(Boolean).join(' · ');
}

export function formatTime(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
}
