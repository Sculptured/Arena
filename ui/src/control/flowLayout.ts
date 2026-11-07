// The process view as a React Flow graph: agents (units) in a column on the left, the world's
// systems (equipment) on the right, one edge per flow, heavier when it carries writes.
import type { Edge, Node } from '@xyflow/react';
import { type Equipment, type Flow, type ProcessView, type Unit, abnormal } from './process';

export const UNIT_W = 232;
export const EQUIP_W = 300;
export const ROW_H = 92;
const GAP_X = 260;

export type UnitNode = Node<{ unit: Unit }, 'unit'>;
export type EquipNode = Node<{ equipment: Equipment }, 'equipment'>;

/** The simple shape that stands for a kind of system. */
export type EquipShape = 'cylinder' | 'box' | 'diamond' | 'hexagon' | 'truck';

export function equipShape(kind: string | undefined): EquipShape {
  if (kind === 'database' || kind === 'lims' || kind === 'folder') return 'cylinder';
  if (kind === 'approvals') return 'diamond';
  if (kind === 'api' || kind === 'mcp' || kind === 'messaging') return 'hexagon';
  if (kind === 'logistics' || kind === 'plant') return 'truck';
  return 'box';
}

/** A flow's line: weight from its writes (reads only: thin and dashed), the alarm colour when abnormal. */
export function flowStroke(f: Flow): { width: number; dashed: boolean; abnormal: boolean } {
  const width = f.writes > 0 ? Math.min(5, 2 + Math.log2(f.writes)) : 1.25;
  return { width, dashed: f.writes === 0, abnormal: abnormal(f) };
}

/** "⚠ 1 refused, 2 errors" on an abnormal flow; nothing on a normal one (its counts are on the nodes). */
export function flowLabel(f: Flow): string {
  if (!abnormal(f)) return '';
  const bits = [f.refused && `${f.refused} refused`, f.errors && `${f.errors} error${f.errors > 1 ? 's' : ''}`].filter(Boolean);
  return `⚠ ${bits.join(', ')}`;
}

/** Both columns centred on each other. */
function column<T>(items: T[], rows: number): { item: T; y: number }[] {
  const offset = ((rows - items.length) * ROW_H) / 2;
  return items.map((item, i) => ({ item, y: offset + i * ROW_H }));
}

export function flowGraph(view: ProcessView): { nodes: (UnitNode | EquipNode)[]; edges: Edge[]; height: number } {
  const rows = Math.max(view.units.length, view.equipment.length, 1);
  const units: UnitNode[] = column(view.units, rows).map(({ item, y }) => ({ id: `u:${item.id}`, type: 'unit', position: { x: 0, y }, data: { unit: item }, width: UNIT_W }));
  const equipment: EquipNode[] = column(view.equipment, rows).map(({ item, y }) => ({ id: `e:${item.part}`, type: 'equipment', position: { x: UNIT_W + GAP_X, y }, data: { equipment: item }, width: EQUIP_W }));
  const known = new Set([...units, ...equipment].map((n) => n.id));
  const edges: Edge[] = view.flows.filter((f) => known.has(`u:${f.agent}`) && known.has(`e:${f.part}`)).map((f) => {
    const s = flowStroke(f);
    return {
      id: `${f.agent}->${f.part}`, source: `u:${f.agent}`, target: `e:${f.part}`, label: flowLabel(f) || undefined,
      style: { strokeWidth: s.width, stroke: s.abnormal ? 'var(--alarm)' : s.dashed ? 'var(--flow)' : 'var(--flow-strong)', strokeDasharray: s.dashed ? '4 3' : undefined },
      labelStyle: { fill: 'var(--alarm)', fontSize: 11, fontWeight: 600 }, labelBgStyle: { fill: 'var(--surface)' },
    };
  });
  return { nodes: [...units, ...equipment], edges, height: rows * ROW_H };
}
