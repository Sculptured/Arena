import { Handle, type NodeProps, Position, ReactFlow, ReactFlowProvider } from '@xyflow/react';
import { useMemo } from 'react';
import { StatusMark, Tag } from '../components/ui/status';
import { cn } from '@/lib/cn';
import { kindWord, systemLine } from './actions';
import { EQUIP_W, type EquipNode, type EquipShape, UNIT_W, type UnitNode, equipShape, flowGraph } from './flowLayout';
import { type Counts, type ProcessView, type UnitState, abnormal, countsLine } from './process';

const STATE: Record<UnitState, { glyph: string; word: string; abnormal: boolean }> = {
  idle: { glyph: '○', word: 'Idle', abnormal: false },
  running: { glyph: '◐', word: 'Running', abnormal: false },
  done: { glyph: '■', word: 'Done', abnormal: false },
  failed: { glyph: '✕', word: 'Failed', abnormal: true },
  automatic: { glyph: '⟳', word: 'Run by Arena', abnormal: false },
};

function Alarm({ c }: { c: Counts }) {
  if (!abnormal(c)) return null;
  const bits = [c.errors && `${c.errors} error${c.errors > 1 ? 's' : ''}`, c.refused && `${c.refused} refused`].filter(Boolean);
  return <StatusMark mark={{ glyph: '⚠', word: bits.join(', '), abnormal: true }} />;
}

/** The equipment's simple shape: a cylinder for data stores, a box for applications, a diamond for approvals. */
function ShapeGlyph({ shape }: { shape: EquipShape }) {
  const common = { fill: 'var(--equip)', stroke: 'var(--equip-line)', strokeWidth: 1.25 };
  return (
    <svg width="22" height="22" viewBox="0 0 22 22" aria-hidden="true" className="shrink-0">
      {shape === 'cylinder' && <><path d="M3 5v12c0 1.7 3.6 3 8 3s8-1.3 8-3V5" {...common} /><ellipse cx="11" cy="5" rx="8" ry="3" {...common} /></>}
      {shape === 'box' && <rect x="2.5" y="4" width="17" height="14" rx="1" {...common} />}
      {shape === 'diamond' && <path d="M11 2 20 11 11 20 2 11Z" {...common} />}
      {shape === 'hexagon' && <path d="M6 3h10l4 8-4 8H6l-4-8Z" {...common} />}
      {shape === 'truck' && <><rect x="1.5" y="6" width="12" height="9" {...common} /><path d="M13.5 9h4l3 3v3h-7Z" {...common} /><circle cx="6" cy="17" r="2" {...common} /><circle cx="16" cy="17" r="2" {...common} /></>}
    </svg>
  );
}

function UnitBox({ data }: NodeProps<UnitNode>) {
  const u = data.unit;
  return (
    <div className={cn('flex flex-col gap-0.5 rounded-md border bg-raised px-2.5 py-1.5 shadow-panel', abnormal(u) ? 'border-alarm' : 'border-line-strong')} style={{ width: UNIT_W }}>
      <span className="truncate text-sm font-medium" title={u.id}>{u.name}</span>
      <span className="flex flex-wrap items-center gap-1.5 text-xs">
        <StatusMark mark={STATE[u.state]} /><span className="font-mono text-2xs text-muted">{countsLine(u)}</span><Alarm c={u} />
      </span>
      <Handle type="source" position={Position.Right} className="!size-1.5 !border-0 !bg-line-strong" isConnectable={false} />
    </div>
  );
}

function EquipBox({ data }: NodeProps<EquipNode>) {
  const e = data.equipment;
  return (
    <div className={cn('flex items-start gap-2 rounded-sm border bg-surface px-2 py-1.5', abnormal(e) ? 'border-alarm' : 'border-equip-line')} style={{ width: EQUIP_W }} title={systemLine(e.system)}>
      <Handle type="target" position={Position.Left} className="!size-1.5 !border-0 !bg-line-strong" isConnectable={false} />
      <ShapeGlyph shape={equipShape(e.system?.kind)} />
      <div className="flex min-w-0 flex-col gap-0.5">
        <span className="flex items-center gap-1.5"><span className="text-sm font-semibold">{kindWord(e.system)}</span><span className="font-mono text-2xs text-muted">{e.part}</span>{e.system?.simulated && <Tag>simulated</Tag>}</span>
        <span className="truncate text-2xs text-muted">{systemLine(e.system)}</span>
        <span className="flex flex-wrap items-center gap-1.5"><span className="font-mono text-2xs text-muted">{countsLine(e)}</span><Alarm c={e} /></span>
      </div>
    </div>
  );
}

const NODE_TYPES = { unit: UnitBox, equipment: EquipBox };

/** Agents as units (left), the world's systems as equipment (right), the action flows between them; writes heavier, abnormal flows in the alarm colour. */
export default function ProcessDiagram({ view }: { view: ProcessView }) {
  const g = useMemo(() => flowGraph(view), [view]);
  return (
    // Narrow screens scroll the diagram sideways rather than shrink it past reading.
    <div className="w-full overflow-x-auto">
      <div className="w-full min-w-[720px]" style={{ height: Math.max(220, g.height + 40) }}>
        <ReactFlowProvider>
          <ReactFlow nodes={g.nodes} edges={g.edges} nodeTypes={NODE_TYPES} fitView fitViewOptions={{ padding: 0.08 }} proOptions={{ hideAttribution: true }}
            nodesDraggable={false} nodesConnectable={false} elementsSelectable={false} zoomOnScroll={false} preventScrolling={false} minZoom={0.4} maxZoom={1.5} />
        </ReactFlowProvider>
      </div>
    </div>
  );
}
