import { CircleHelp, Minus, Plus, Radio } from 'lucide-react';
import { type CSSProperties, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { CHECK_LANE, type LaneRow, type Mark, type Span, WORLD_LANE } from '../control/lanes';
import { Button } from '../components/ui/button';
import { Checkbox } from '../components/ui/input';
import { Popover, PopoverContent, PopoverTrigger } from '../components/ui/popover';
import { cn } from '@/lib/cn';
import { modelBadge } from '../board/board';
import type { Team } from '../types';
import type { Pause } from './flow';
import { LANE_H, type Layout, type Placed, type Shape, layoutTimeline, shapeOf, shortLabel } from './timelineLayout';
import { MAX_ZOOM, MIN_ZOOM, type View, clampScroll, endScroll, onWheel, stepZoom, trackWidth } from './zoom';
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
      <div className="border-b border-line px-2 text-2xs leading-[21px] font-medium text-muted" style={{ height: RULER_H }}>Agent</div>
      {lanes.map((l) => {
        const agent = team.agents.find((a) => a.id === l.id);
        const model = agent ? modelBadge(agent) : l.id === WORLD_LANE ? 'Links and world events' : l.id === CHECK_LANE ? 'Scoring' : '';
        return (
          <div key={l.id} className="flex flex-col justify-center border-b border-line/60 px-2 last:border-b-0" style={{ height: LANE_H }} title={model ? `${l.id} · ${model}` : l.id}>
            <span className="truncate text-xs font-medium">{agent?.name ?? l.name}</span>
            {model && <span className="truncate text-2xs text-faint">{model}</span>}
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

export interface TimelineProps {
  lanes: LaneRow[];
  team: Team;
  span: Span | null;
  selected: number | null;
  onSelect: (seq: number) => void;
  models: boolean;
  onModels: (show: boolean) => void;
  /** A live run: the track follows the newest step until the user scrolls or zooms away. */
  live?: boolean;
  /** Pauses, drawn as bands across the lanes. */
  pauses?: Pause[];
}

/** The track's zoom and scroll, applied to the box, and whether it follows the newest step. */
function useTrackView(el: HTMLDivElement | null, viewW: number, minW: number, live: boolean) {
  const [view, setView] = useState<View>({ zoom: 1, scroll: 0 });
  const [follow, setFollow] = useState(true);
  const trackW = trackWidth(viewW, view.zoom, minW);
  const following = live && follow;
  // While following, the end of the track (the newest step) stays in view.
  const scroll = following ? endScroll(trackW, viewW) : clampScroll(view.scroll, trackW, viewW);
  useLayoutEffect(() => { if (el && Math.abs(el.scrollLeft - scroll) > 0.5) el.scrollLeft = scroll; });
  // Kept in step while following, so the view stays where it was when the run ends.
  useLayoutEffect(() => { if (following && view.scroll !== scroll) setView((v) => ({ ...v, scroll })); }, [following, scroll, view.scroll]);
  const latest = useRef({ view: { ...view, scroll }, viewW, minW });
  latest.current = { view: { ...view, scroll }, viewW, minW };
  /** A move the user made: it stops following. */
  const move = useCallback((next: View) => { setView(next); setFollow(false); }, []);
  return { view: { ...view, scroll }, trackW, following, move, latest, follow: () => setFollow(true), setFollow };
}

/** Ctrl/⌘ + wheel or a pinch zooms around the cursor, the wheel scrolls sideways, a drag on empty space pans. */
function useMouse(el: HTMLDivElement | null, labelW: number, t: ReturnType<typeof useTrackView>) {
  const { latest, move } = t;
  useEffect(() => {
    if (!el) return;
    const onWheelEvent = (e: WheelEvent) => {
      const { view, viewW, minW } = latest.current;
      const cursor = Math.max(0, e.clientX - el.getBoundingClientRect().left - labelW);
      const next = onWheel(e, view, cursor, viewW, minW);
      if (!next) return;
      e.preventDefault();
      move(next);
    };
    el.addEventListener('wheel', onWheelEvent, { passive: false });
    return () => el.removeEventListener('wheel', onWheelEvent);
  }, [el, labelW, latest, move]);
  const drag = useRef<{ x: number; scroll: number } | null>(null);
  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest('button')) return;
    drag.current = { x: e.clientX, scroll: latest.current.view.scroll };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  };
  const onPointerMove = (e: React.PointerEvent) => {
    if (!drag.current) return;
    const { view, viewW, minW } = latest.current;
    const trackW = trackWidth(viewW, view.zoom, minW);
    move({ ...view, scroll: clampScroll(drag.current.scroll - (e.clientX - drag.current.x), trackW, viewW) });
  };
  const onPointerUp = () => { drag.current = null; };
  return { onPointerDown, onPointerMove, onPointerUp, onPointerCancel: onPointerUp };
}

/** A scrollbar or touch scroll the user made also stops following. */
function useNativeScroll(el: HTMLDivElement | null, t: ReturnType<typeof useTrackView>) {
  const { latest, move } = t;
  useEffect(() => {
    if (!el) return;
    const onScroll = () => {
      const { view } = latest.current;
      if (Math.abs(el.scrollLeft - view.scroll) > 1) move({ ...view, scroll: el.scrollLeft });
    };
    el.addEventListener('scroll', onScroll, { passive: true });
    return () => el.removeEventListener('scroll', onScroll);
  }, [el, latest, move]);
}

/** Scrolls the selected mark into the middle of the view when it is out of sight. */
function useShowSelected(selected: number | null, layout: Layout | null, t: ReturnType<typeof useTrackView>) {
  const { latest, move } = t;
  useEffect(() => {
    const x = layout?.placed.find((p) => p.mark.seq === selected)?.x;
    const { view, viewW, minW } = latest.current;
    if (x == null || (x >= view.scroll + 24 && x <= view.scroll + viewW - 24)) return;
    move({ ...view, scroll: clampScroll(x - viewW / 2, trackWidth(viewW, view.zoom, minW), viewW) });
  }, [selected, layout, latest, move]);
}

function Toolbar({ t, viewW, minW, models, onModels, live }: {
  t: ReturnType<typeof useTrackView>; viewW: number; minW: number; models: boolean; onModels: (s: boolean) => void; live: boolean;
}) {
  const { view } = t;
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-line px-3 py-1.5 text-xs text-muted">
      <Legend />
      <span className="hidden text-faint sm:inline">Scroll to move · Ctrl + scroll or pinch to zoom · drag to pan</span>
      {live && !t.following && <Button size="sm" onClick={t.follow} title="Follow the newest step"><Radio />Follow</Button>}
      <label className="ml-auto flex items-center gap-1.5"><Checkbox checked={models} onChange={(e) => onModels(e.target.checked)} />Model calls</label>
      <div className="flex items-center gap-0.5" role="group" aria-label="Zoom">
        <Button size="icon" variant="ghost" className="size-6" aria-label="Zoom out" disabled={view.zoom <= MIN_ZOOM} onClick={() => t.move(stepZoom(view, -1, viewW, minW))}><Minus /></Button>
        <Button size="sm" variant="ghost" className="w-12 font-mono" onClick={() => t.move({ zoom: 1, scroll: 0 })} title="Fit the run to the width">{view.zoom <= 1.01 ? 'Fit' : `${view.zoom.toFixed(view.zoom < 10 ? 1 : 0)}×`}</Button>
        <Button size="icon" variant="ghost" className="size-6" aria-label="Zoom in" disabled={view.zoom >= MAX_ZOOM} onClick={() => t.move(stepZoom(view, 1, viewW, minW))}><Plus /></Button>
      </div>
    </div>
  );
}

function Legend() {
  const item = (shape: Shape, word: string) => (
    <span className="inline-flex items-center gap-1.5"><span className="grid size-3 place-items-center"><span className={cn('block', SHAPE[shape])} /></span>{word}</span>
  );
  return (
    <Popover>
      <PopoverTrigger asChild><Button size="icon" variant="ghost" className="size-6" aria-label="What the marks mean"><CircleHelp /></Button></PopoverTrigger>
      <PopoverContent className="flex w-56 flex-col gap-1.5 text-xs">
        {item('read', 'Read')}{item('write', 'Write')}{item('gate-write', 'Approval')}{item('message', 'Message')}{item('model', 'Model call')}
        <span className="inline-flex items-center gap-1.5 text-alarm"><span className="grid size-3 place-items-center"><span className={cn('block', SHAPE.abnormal)} /></span>Refused or error</span>
        <span className="inline-flex items-center gap-1.5"><span className="h-3 w-3 bg-sunken ring-1 ring-line" />Paused</span>
      </PopoverContent>
    </Popover>
  );
}

/** Pauses as shaded bands across the lanes. */
function Bands({ layout, height }: { layout: Layout; height: number }) {
  return layout.bands.map((b) => (
    <div key={b.x1} className="pointer-events-none absolute bg-sunken/80 ring-1 ring-line" style={{ left: b.x1, width: b.x2 - b.x1, top: RULER_H, height }} title="Paused">
      <span className="absolute top-1 left-1 text-2xs text-faint">Paused</span>
    </div>
  ));
}

/** Swim lanes per agent on one time axis: every action labelled by system kind and read/write, messages drawn between lanes. */
export default function ActionTimeline({ lanes, team, span, selected, onSelect, models, onModels, live = false, pauses = [] }: TimelineProps) {
  const box = useWidth();
  const labelW = box.width && box.width < 640 ? 112 : 184;
  // A phone gets a track wider than the screen, to scroll; a desktop fits the run at 1×.
  const minW = box.width < 640 ? 640 : 240;
  const viewW = Math.max(0, box.width - labelW);
  const t = useTrackView(box.ref.current, viewW, minW, live);
  const layout = useMemo(() => (span ? layoutTimeline(lanes, span, t.trackW, models, pauses) : null), [lanes, span, t.trackW, models, pauses]);
  const mouse = useMouse(box.ref.current, labelW, t);
  useNativeScroll(box.ref.current, t);
  useShowSelected(selected, layout, t);
  const height = lanes.length * LANE_H;
  return (
    <div className="flex min-w-0 flex-col">
      <Toolbar t={t} viewW={viewW} minW={minW} models={models} onModels={onModels} live={live} />
      <div ref={box.ref} className="relative flex min-w-0 cursor-grab touch-pan-y overflow-x-auto overscroll-x-contain select-none active:cursor-grabbing" {...mouse}>
        <LaneLabels lanes={lanes} team={team} width={labelW} />
        {layout && (
          <div className="relative shrink-0" style={{ width: t.trackW, height: RULER_H + height }}>
            <Ruler layout={layout} />
            <Bands layout={layout} height={height} />
            <Wires layout={layout} lanes={lanes.length} width={t.trackW} selected={selected} />
            {layout.placed.map((p) => <MarkButton key={p.mark.seq} p={p} selected={selected === p.mark.seq} onSelect={onSelect} />)}
          </div>
        )}
      </div>
    </div>
  );
}
