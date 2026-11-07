import type { Branch } from '../types';

/** Branches in tree order with their depth (parent before children, oldest first). */
export function branchTree(branches: Branch[]): { branch: Branch; depth: number }[] {
  const ids = new Set(branches.map((b) => b.id));
  const kids = new Map<string | null, Branch[]>();
  for (const b of [...branches].sort((x, y) => x.created_at.localeCompare(y.created_at))) {
    const key = b.parent_id && ids.has(b.parent_id) ? b.parent_id : null;
    kids.set(key, [...(kids.get(key) ?? []), b]);
  }
  const out: { branch: Branch; depth: number }[] = [];
  const walk = (key: string | null, depth: number) => {
    for (const b of kids.get(key) ?? []) {
      out.push({ branch: b, depth });
      walk(b.id, depth + 1);
    }
  };
  walk(null, 0);
  return out;
}
