import { type Edge, Handle, MarkerType, type Node, type NodeProps, Panel, Position, ReactFlow, type ReactFlowInstance, ReactFlowProvider, useReactFlow } from '@xyflow/react';
import { Maximize2, Minus, Plus } from 'lucide-react';
import { useMemo, useRef } from 'react';
import { Button } from '../components/ui/button';
import { Tag } from '../components/ui/status';
import { cn } from '@/lib/cn';
import type { AgentSpec, Runner, Team } from '../types';
import { agentModelLine } from '../providers';
import { type BoardGroup, CARD_H, CARD_W, boardSize, jobLine, layoutBoard, openingView } from './board';

type CardData = { agent: AgentSpec; decides: string[]; selected: boolean };
type ZoneData = { group: BoardGroup };
type CardNode = Node<CardData, 'card'>;
type ZoneNode = Node<ZoneData, 'zone'>;

const HANDLE = '!size-1.5 !border-0 !bg-line-strong';

function AgentCard({ data }: NodeProps<CardNode>) {
  const { agent: a, decides, selected } = data;
  return (
    <div className={cn('flex cursor-pointer flex-col gap-1 rounded-md border bg-raised px-2.5 py-2 shadow-panel hover:border-line-strong', selected ? 'border-accent ring-2 ring-accent' : 'border-line')}
      style={{ width: CARD_W, height: CARD_H }} title={`${a.name}: click for its prompt`}>
      <Handle type="target" position={Position.Left} className={HANDLE} isConnectable={false} />
      <span className="truncate text-sm font-medium">{a.name}</span>
      <span className="line-clamp-2 text-xs leading-4 text-muted">{jobLine(a) || 'No prompt yet.'}</span>
      <span className="mt-auto flex min-w-0 flex-wrap gap-1 overflow-hidden" style={{ maxHeight: 18 }}>
        <Tag>{agentModelLine(a)}</Tag>
        {a.read_only && <Tag>Read-only</Tag>}
        {decides.map((d) => <Tag key={d} title="A decision this agent owns">◆ {d}</Tag>)}
      </span>
      <Handle type="source" position={Position.Right} className={HANDLE} isConnectable={false} />
    </div>
  );
}

function Zone({ data }: NodeProps<ZoneNode>) {
  const g = data.group;
  return (
    <div className="rounded-lg border border-dashed border-line-strong bg-sunken/60" style={{ width: g.w, height: g.h }}>
      <span className="block px-2.5 pt-1.5 text-xs font-medium text-muted">{g.label}</span>
    </div>
  );
}

const NODE_TYPES = { card: AgentCard, zone: Zone };

/** The board as React Flow nodes and edges: group zones behind, cards on top, arrows for who hands work to whom. */
function graph(team: Team, runner: Runner, selected: string | null): { nodes: (CardNode | ZoneNode)[]; edges: Edge[] } {
  const b = layoutBoard(team, runner);
  const zones: ZoneNode[] = b.groups.map((g) => ({ id: `zone:${g.id}`, type: 'zone', position: { x: g.x, y: g.y }, data: { group: g }, selectable: false, zIndex: -1 }));
  const cards: CardNode[] = b.cards.map((c) => ({
    id: c.agent.id, type: 'card', position: { x: c.x, y: c.y },
    data: { agent: c.agent, decides: team.board?.decides?.[c.agent.id] ?? [], selected: c.agent.id === selected },
  }));
  const edges: Edge[] = b.links.map((l) => ({
    id: `${l.from}->${l.to}`, source: l.from, target: l.to, type: 'smoothstep',
    style: { stroke: 'var(--line-strong)', strokeWidth: 1.25 }, markerEnd: { type: MarkerType.ArrowClosed, color: 'var(--line-strong)', width: 14, height: 14 },
  }));
  return { nodes: [...zones, ...cards], edges };
}

const ZOOM_MS = 150;

/** −, Fit and +, and Expand: the same feel as the timeline. */
function Controls({ onExpand }: { onExpand?: () => void }) {
  const rf = useReactFlow();
  return (
    <Panel position="top-right" className="!m-2 flex items-center gap-0.5 rounded-md border border-line bg-raised p-0.5 opacity-0 shadow-panel transition-opacity group-hover/board:opacity-100 focus-within:opacity-100">
      <Button size="icon" variant="ghost" className="size-6" aria-label="Zoom out" onClick={() => rf.zoomOut({ duration: ZOOM_MS })}><Minus /></Button>
      <Button size="sm" variant="ghost" onClick={() => rf.fitView({ padding: 0.08, duration: ZOOM_MS })} title="Show the whole board">Fit</Button>
      <Button size="icon" variant="ghost" className="size-6" aria-label="Zoom in" onClick={() => rf.zoomIn({ duration: ZOOM_MS })}><Plus /></Button>
      {onExpand && <Button size="sm" variant="ghost" onClick={onExpand} title="Open the board full screen"><Maximize2 />Expand</Button>}
    </Panel>
  );
}

/**
 * A team as a read-only Sculptured-style board: cards grouped by role, "hands work to" arrows.
 * It opens readable (a big board from its corner, to pan); the wheel pans, Ctrl/⌘ + wheel or a
 * pinch zooms, a drag pans. Clicking a card picks it.
 */
export default function TeamBoard({ team, runner, selected, onSelect, onExpand, className }: {
  team: Team; runner: Runner; selected: string | null; onSelect: (id: string | null) => void; onExpand?: () => void; className?: string;
}) {
  const g = useMemo(() => graph(team, runner, selected), [team, runner, selected]);
  const box = useRef<HTMLDivElement>(null);
  // A new team or runner opens afresh.
  const fitKey = `${team.id}:${team.agents.map((a) => a.id).join(',')}:${runner}`;
  const open = (rf: ReactFlowInstance<CardNode | ZoneNode>) => {
    const el = box.current;
    if (el) rf.setViewport(openingView(boardSize(layoutBoard(team, runner)), el.clientWidth, el.clientHeight));
  };
  return (
    <div ref={box} className={cn('group/board h-72 w-full', className)}>
      <ReactFlowProvider key={fitKey}>
        <ReactFlow nodes={g.nodes} edges={g.edges} nodeTypes={NODE_TYPES} onInit={open} proOptions={{ hideAttribution: true }}
          nodesDraggable={false} nodesConnectable={false} elementsSelectable={false} panOnScroll zoomOnScroll={false} zoomOnPinch minZoom={0.2} maxZoom={1.5}
          onNodeClick={(_, n) => { if (n.type === 'card') onSelect(n.id === selected ? null : n.id); }} onPaneClick={() => onSelect(null)}>
          <Controls onExpand={onExpand} />
        </ReactFlow>
      </ReactFlowProvider>
    </div>
  );
}
