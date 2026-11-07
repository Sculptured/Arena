import { useMemo } from 'react';
import { StatusMark } from '../components/ui/status';
import { cn } from '@/lib/cn';
import { type TwinLayout, type ZoneBox, fitsSideBySide, splitAreas, stableHeight, twinLayout } from './layout';
import { TwinTile, ZoneGlyph } from './TwinTile';
import { type ViewChanges, itemsById, statusMark } from './twin';
import type { TwinArea, TwinFlow, TwinView } from './types';
import { useWidth } from './useWorld';

function ZoneHead({ area, className }: { area: TwinArea; className?: string }) {
  const bad = area.items.filter((i) => i.abnormal).length;
  return (
    <div className={cn('flex h-[30px] items-center gap-1.5 px-2', className)}>
      <ZoneGlyph kind={area.kind} />
      <span className="truncate text-2xs font-semibold text-muted">{area.label}</span>
      <span className="font-mono text-2xs text-faint">{area.items.length}</span>
      {bad > 0 && <span className="ml-auto font-mono text-2xs font-semibold text-alarm">⚠ {bad}</span>}
    </div>
  );
}

function Zone({ z, height }: { z: ZoneBox; height: number }) {
  return (
    <div className={cn('absolute rounded-md border', z.rail ? 'border-dashed border-line-strong bg-surface' : 'border-line bg-sunken')} style={{ left: z.x, top: z.y, width: z.w, height }}>
      <ZoneHead area={z.area} />
      {z.area.items.length === 0 && <p className="px-2 pt-1 text-2xs text-faint">{z.rail ? 'No changes proposed yet.' : 'Nothing here.'}</p>}
    </div>
  );
}

function Arrows({ l, fresh }: { l: TwinLayout; fresh: Set<string> }) {
  const color = (f: TwinFlow) => (f.abnormal ? 'var(--alarm)' : fresh.has(f.id) ? 'var(--accent)' : 'var(--flow-strong)');
  return (
    <svg className="pointer-events-none absolute inset-0" width={l.width} height={l.height} aria-hidden="true">
      <defs>
        {['flow', 'alarm', 'accent'].map((k) => (
          <marker key={k} id={`twin-head-${k}`} viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto">
            <path d="M0,0 L8,4 L0,8 Z" fill={k === 'flow' ? 'var(--flow-strong)' : `var(--${k})`} />
          </marker>
        ))}
      </defs>
      {l.arrows.map((a) => (
        <g key={a.flow.id}>
        <path d={a.d} fill="none" stroke={color(a.flow)} strokeWidth={a.flow.abnormal || fresh.has(a.flow.id) ? 2.25 : 1.5}
          strokeDasharray={a.flow.abnormal ? '6 3' : undefined} markerEnd={`url(#twin-head-${a.flow.abnormal ? 'alarm' : fresh.has(a.flow.id) ? 'accent' : 'flow'})`}>
          <title>{`${a.flow.label}: ${a.flow.status}`}</title>
        </path>
        {a.label && <text x={a.label.x} y={a.label.y} className={cn('text-[10px]', a.flow.abnormal ? 'fill-alarm font-semibold' : fresh.has(a.flow.id) ? 'fill-accent font-semibold' : 'fill-muted')}>{a.flow.abnormal ? `⚠ ${a.flow.id} · ${a.flow.status}` : `${a.flow.label} · ${a.flow.id}`}</text>}
        </g>
      ))}
    </svg>
  );
}

/** Areas side by side in process order, the rail on the right, flows as arrows between tiles. */
function Wide({ view, changes, width, all }: { view: TwinView; changes: ViewChanges; width: number; all: TwinView[] }) {
  const changed = useMemo(() => new Map(changes.items.filter((c) => c.kind !== 'removed').map((c) => [c.id, c.short])), [changes]);
  const l = useMemo(() => twinLayout(view, width, new Set(changed.keys())), [view, width, changed]);
  const items = useMemo(() => itemsById(view), [view]);
  const fresh = useMemo(() => new Set(changes.flows.map((f) => f.id)), [changes]);
  const height = useMemo(() => Math.max(l.height, stableHeight(all, width)), [l.height, all, width]);
  return (
    <div className="relative" style={{ height }}>
      {l.zones.map((z) => <Zone key={z.area.id} z={z} height={height} />)}
      <Arrows l={l} fresh={fresh} />
      {l.tiles.map((t) => {
        const it = items.get(t.id);
        return it ? <TwinTile key={t.id} item={it} change={changed.get(t.id)} className="absolute" style={{ left: t.x, top: t.y, width: t.w, height: t.h }} /> : null;
      })}
    </div>
  );
}

function FlowList({ view, fresh }: { view: TwinView; fresh: Set<string> }) {
  const items = itemsById(view);
  if (!view.flows.length) return null;
  return (
    <ul className="flex flex-col gap-1 border-t border-line px-2 py-2">
      {view.flows.map((f) => (
        <li key={f.id} className={cn('flex flex-wrap items-center gap-x-1.5 text-xs', fresh.has(f.id) && 'font-semibold')}>
          <span className="font-mono">{items.get(f.from)?.label ?? f.from}</span><span className="text-faint">→</span>
          <span>{items.get(f.to)?.label ?? f.to}</span><span className="text-muted">· {f.label}</span><StatusMark mark={statusMark(f)} />
        </li>
      ))}
    </ul>
  );
}

/** Phones: each area stacked, its tiles two by two, flows listed under the customers. */
function Stacked({ view, changes }: { view: TwinView; changes: ViewChanges }) {
  const { main, rail } = splitAreas(view);
  const changed = new Map(changes.items.map((c) => [c.id, c.short]));
  const fresh = new Set(changes.flows.map((f) => f.id));
  return (
    <div className="flex flex-col gap-2">
      {[...main, ...rail].map((a) => (
        <section key={a.id} className={cn('rounded-md border', a.kind === 'approvals' ? 'border-dashed border-line-strong' : 'border-line bg-sunken')}>
          <ZoneHead area={a} />
          <div className="grid grid-cols-2 gap-1.5 px-2 pb-2">
            {a.items.map((it) => <TwinTile key={it.id} item={it} change={changed.get(it.id)} />)}
            {!a.items.length && <p className="col-span-2 text-2xs text-faint">Nothing here yet.</p>}
          </div>
          {a.kind === 'customers' && <FlowList view={view} fresh={fresh} />}
        </section>
      ))}
    </div>
  );
}

/** The twin as an HMI overview: wide screens side by side with arrows, narrow ones stacked. */
export default function TwinCanvas({ view, changes, all = [view] }: { view: TwinView; changes: ViewChanges; all?: TwinView[] }) {
  const { ref, width } = useWidth<HTMLDivElement>();
  const wide = width > 0 && fitsSideBySide(view, width);
  return (
    <div ref={ref} className="min-w-0">
      {width > 0 && (wide ? <Wide view={view} changes={changes} width={width} all={all} /> : <Stacked view={view} changes={changes} />)}
    </div>
  );
}
