import { Minus, Plus } from 'lucide-react';
import { type CSSProperties, useEffect, useMemo, useRef, useState } from 'react';
import { CHECK_LANE, type LaneRow, type Mark, type Span, WORLD_LANE } from '../control/lanes';
import { Button } from '../components/ui/button';
import { Checkbox } from '../components/ui/input';
import { cn } from '@/lib/cn';
import { agentModelLine } from '../providers';
import type { Team } from '../types';
import { LANE_H, type Layout, type Placed, type Shape, layoutTimeline, shapeOf, shortLabel } from './timelineLayout';

const ZOOMS = [1, 2, 4, 8, 16, 32];
const RULER_H = 22;

/** Each shape's look: hollow reads, solid (heavier) writes, diamonds for approvals, a triangle in alarm colour for refusals and errors. */
const SHAPE: Record<Shape, string> = {
  read: 'size-2.5 rounded-full border-[1.5px] border-flow-strong bg-surface',
  write: 'size-2.5 rounded-[1px] bg-flow-strong',
  call: 'size-2.5 rounded-[1px] border-[1.5px] border-flow-strong bg-surface',
  'gate-read': 'size-2.5 rotate-45 border-[1.5px] border-flow-strong bg-surface',
  'gate-write': 'size-2.5 rotate-45 bg-flow-strong',
  abnormal: 'size-3.5 bg-alarm [clip-path:polygon(50%_0,100%_100%,0_100%)]',
  message: 'size-2 rounded-full bg-muted',
  model: 'h-2.5 w-[2px] rounded-none bg-line-strong',
  start: 'h-3 w-[3px] bg-muted',
  end: 'h-3 w-[3px] bg-muted',
  'end-failed': 'size-3.5 bg-alarm [clip-path:polygon(50%_0,100%_100%,0_100%)]',
  'check-pass': 'size-3 rounded-full bg-muted',
  'check-fail': 'size-3.5 bg-alarm [clip-path:polygon(50%_0,100%_100%,0_100%)]',
  world: 'size-2 rotate-45 border border-muted bg-surface',
};

const hint = (m: Mark) => `step ${m.seq} · ${m.label}${m.detail ? ` · ${m.detail.slice(0, 160)}` : ''}${m.saved ? ' · saved after this step' : ''}${m.inherited ? ' (inherited from the parent run)' : ''}`;

/** The label next to a mark: the full action word when selected, the system kind when there is room. */
function textOf(p: Placed, selected: boolean): string {
  const action = p.mark.kind === 'action' || p.mark.kind === 'interlock';
  if (selected && action) return p.mark.label;
  return p.labelled || selected ? shortLabel(p.mark) : '';
}

const ALARM_SHAPES: Shape[] = ['abnormal', 'check-fail', 'end-failed'];

function labelClass(p: Placed, shape: Shape, selected: boolean): string {
  const tone = ALARM_SHAPES.includes(shape) ? 'font-medium text-alarm' : p.mark.effect === 'write' ? 'font-medium text-fg' : 'text-muted';
  return cn('rounded-sm px-0.5 text-2xs whitespace-nowrap', tone, selected && 'bg-raised text-fg shadow-pop ring-1 ring-accent');
}

function MarkButton({ p, selected, onSelect }: { p: Placed; selected: boolean; onSelect: (seq: number) => void }) {
  const shape = shapeOf(p.mark);
  const text = textOf(p, selected);
  const style: CSSProperties = { left: p.x, top: RULER_H + p.lane * LANE_H + LANE_H / 2 };
  return (
    <button data-seq={p.mark.seq} title={hint(p.mark)} aria-label={hint(p.mark)} aria-pressed={selected} onClick={() => onSelect(p.mark.seq)}
      className={cn('group absolute flex items-center gap-1 rounded-sm outline-none', p.mark.inherited && 'opacity-40', selected ? 'z-20' : 'z-10')}
      style={{ ...style, transform: 'translate(-6px, -50%)' }}>
      <span className="grid size-3 place-items-center">
        <span className={cn('block', SHAPE[shape], selected && 'ring-2 ring-accent ring-offset-2 ring-offset-surface')} />
      </span>
      {text && <span className={labelClass(p, shape, selected)}>{text}</span>}
    </button>
  );
}

function Ruler({ layout }: { layout: Layout }) {
  return (
    <div className="absolute inset-x-0 top-0 border-b border-line" style={{ height: RULER_H }}>
      {layout.ticks.map((t) => (
        <span key={t.x} className="absolute top-0 h-full border-l border-line pl-1 font-mono text-2xs leading-[22px] text-faint" style={{ left: t.x }}>{t.label}</span>
      ))}
    </div>
  );
}

/** Lane guides, each lane's active span, and the connectors of messages and handoffs between lanes. */
function Wires({ layout, lanes, width, selected }: { layout: Layout; lanes: number; width: number; selected: number | null }) {
  const y = (lane: number) => RULER_H + lane * LANE_H + LANE_H / 2;
  return (
    <svg className="pointer-events-none absolute top-0 left-0" width={width} height={RULER_H + lanes * LANE_H} aria-hidden="true">
      {layout.ticks.map((t) => <line key={t.x} x1={t.x} x2={t.x} y1={RULER_H} y2={RULER_H + lanes * LANE_H} stroke="var(--line)" strokeDasharray="2 4" />)}
      {layout.bars.map((b) => <line key={b.lane} x1={b.x1} x2={b.x2} y1={y(b.lane)} y2={y(b.lane)} stroke="var(--line-strong)" strokeWidth={1} />)}
      {layout.connectors.map((c) => {
        const down = c.to > c.from;
        const y2 = y(c.to) + (down ? -7 : 7);
        const on = selected === c.seq;
        return (
          <g key={c.seq} stroke={on ? 'var(--accent)' : 'var(--muted)'} fill={on ? 'var(--accent)' : 'var(--muted)'} strokeWidth={on ? 1.5 : 1}>
            <line x1={c.x} x2={c.x} y1={y(c.from)} y2={y2} />
            <path d={down ? `M${c.x - 3.5} ${y2 - 5} L${c.x + 3.5} ${y2 - 5} L${c.x} ${y2} Z` : `M${c.x - 3.5} ${y2 + 5} L${c.x + 3.5} ${y2 + 5} L${c.x} ${y2} Z`} stroke="none" />
          </g>
        );
      })}
    </svg>
  );
}

function LaneLabels({ lanes, team, width }: { lanes: LaneRow[]; team: Team; width: number }) {
  return (
    <div className="sticky left-0 z-30 shrink-0 border-r border-line bg-surface" style={{ width }}>
      <div className="border-b border-line px-2 text-2xs leading-[21px] font-medium tracking-wide text-muted uppercase" style={{ height: RULER_H }}>Agent</div>
      {lanes.map((l) => {
        const agent = team.agents.find((a) => a.id === l.id);
        const model = agent ? agentModelLine(agent) : l.id === WORLD_LANE ? 'links and world events' : l.id === CHECK_LANE ? 'scoring' : '';
        return (
          <div key={l.id} className="flex flex-col justify-center border-b border-line/60 px-2 last:border-b-0" style={{ height: LANE_H }} title={model ? `${l.id} · ${model}` : l.id}>
            <span className="truncate text-xs font-medium">{agent?.name ?? l.name}</span>
            {model && <span className="truncate font-mono text-2xs text-faint">{model}</span>}
          </div>
        );
      })}
    </div>
  );
}

function useWidth() {
  const ref = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(el.clientWidth));
    ro.observe(el);
    setWidth(el.clientWidth);
    return () => ro.disconnect();
  }, []);
  return { ref, width };
}

/** Keeps the selected mark in view as the selection moves. */
function useFollow(box: React.RefObject<HTMLDivElement | null>, selected: number | null) {
  useEffect(() => {
    if (selected == null) return;
    box.current?.querySelector(`[data-seq="${selected}"]`)?.scrollIntoView({ block: 'nearest', inline: 'center' });
  }, [box, selected]);
}

export interface TimelineProps {
  lanes: LaneRow[];
  team: Team;
  span: Span | null;
  selected: number | null;
  onSelect: (seq: number) => void;
  models: boolean;
  onModels: (show: boolean) => void;
}

function Toolbar({ zoom, setZoom, models, onModels }: { zoom: number; setZoom: (z: number) => void; models: boolean; onModels: (s: boolean) => void }) {
  const i = ZOOMS.indexOf(zoom);
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-line px-3 py-1.5 text-xs text-muted">
      <Legend />
      <label className="ml-auto flex items-center gap-1.5"><Checkbox checked={models} onChange={(e) => onModels(e.target.checked)} />Model calls</label>
      <div className="flex items-center gap-0.5" role="group" aria-label="Zoom">
        <Button size="icon" variant="ghost" className="size-6" aria-label="Zoom out" disabled={i <= 0} onClick={() => setZoom(ZOOMS[i - 1])}><Minus /></Button>
        <Button size="sm" variant="ghost" className="w-10 font-mono" onClick={() => setZoom(1)} title="Fit the run to the width">{zoom === 1 ? 'fit' : `${zoom}×`}</Button>
        <Button size="icon" variant="ghost" className="size-6" aria-label="Zoom in" disabled={i >= ZOOMS.length - 1} onClick={() => setZoom(ZOOMS[i + 1])}><Plus /></Button>
      </div>
    </div>
  );
}

function Legend() {
  const item = (shape: Shape, word: string) => (
    <span className="inline-flex items-center gap-1"><span className="grid size-3 place-items-center"><span className={cn('block', SHAPE[shape])} /></span>{word}</span>
  );
  return (
    <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
      {item('read', 'read')}{item('write', 'write')}{item('gate-write', 'approval')}{item('message', 'message')}{item('model', 'model call')}
      <span className="inline-flex items-center gap-1 text-alarm">{<span className="grid size-3 place-items-center"><span className={cn('block', SHAPE.abnormal)} /></span>}refused / error</span>
    </span>
  );
}

/** Swim lanes per agent on one time axis: every action labelled by system kind and read/write, messages drawn between lanes. */
export default function ActionTimeline({ lanes, team, span, selected, onSelect, models, onModels }: TimelineProps) {
  const [zoom, setZoom] = useState(1);
  const box = useWidth();
  useFollow(box.ref, selected);
  const labelW = box.width && box.width < 640 ? 112 : 184;
  // A phone gets a track wider than the screen, to scroll; a desktop fits the run at 1×.
  const trackW = Math.max(box.width < 640 ? 640 : 240, (box.width - labelW) * zoom);
  const layout = useMemo(() => (span ? layoutTimeline(lanes, span, trackW, models) : null), [lanes, span, trackW, models]);
  return (
    <div className="flex min-w-0 flex-col">
      <Toolbar zoom={zoom} setZoom={setZoom} models={models} onModels={onModels} />
      <div ref={box.ref} className="relative flex min-w-0 overflow-x-auto overscroll-x-contain">
        <LaneLabels lanes={lanes} team={team} width={labelW} />
        {layout && (
          <div className="relative shrink-0" style={{ width: trackW, height: RULER_H + lanes.length * LANE_H }}>
            <Ruler layout={layout} />
            <Wires layout={layout} lanes={lanes.length} width={trackW} selected={selected} />
            {layout.placed.map((p) => <MarkButton key={p.mark.seq} p={p} selected={selected === p.mark.seq} onSelect={onSelect} />)}
          </div>
        )}
      </div>
    </div>
  );
}
