import { ChevronLeft, ChevronRight, Pause, Play, Undo2 } from 'lucide-react';
import { type PointerEvent, useRef } from 'react';
import { Link } from 'react-router-dom';
import { Button } from '../components/ui/button';
import { Kbd } from '../components/ui/kbd';
import { cn } from '@/lib/cn';
import { type Happening, type Track, type TrackMark, nearestStop, trackAt } from './scrub';
import type { Rewind } from './page';
import type { ItemChange } from './twin';

const MARK: Record<TrackMark['kind'], { glyph: string; word: string; cls: string }> = {
  disruption: { glyph: '▼', word: 'disruption', cls: 'text-warn' },
  refusal: { glyph: '▲', word: 'refused', cls: 'text-alarm' },
  fork: { glyph: '┃', word: 'fork', cls: 'text-muted' },
};

const pct = (seq: number, t: Track) => `${trackAt(seq, t) * 100}%`;

/** The track: a tick per view, marks above it, the part played in the accent, and the thumb. Click or drag to scrub. */
function TrackBar({ track, current, onStep }: { track: Track; current: number | null; onStep: (seq: number) => void }) {
  const box = useRef<HTMLDivElement>(null);
  const pick = (e: PointerEvent<HTMLDivElement>) => {
    const r = box.current?.getBoundingClientRect();
    if (!r || r.width === 0) return;
    const s = nearestStop(track.stops, (e.clientX - r.left) / r.width, track);
    if (s != null) onStep(s);
  };
  return (
    <div className="px-2">
      <div ref={box} className="relative h-10 cursor-pointer touch-none select-none" onPointerDown={(e) => { e.currentTarget.setPointerCapture(e.pointerId); pick(e); }}
        onPointerMove={(e) => { if (e.buttons === 1) pick(e); }}>
        <div className="absolute inset-x-0 top-[26px] h-0.5 rounded-full bg-line-strong" />
        {current != null && <div className="absolute left-0 top-[26px] h-0.5 rounded-full bg-accent" style={{ width: pct(current, track) }} />}
        {track.stops.map((s) => <div key={s} className={cn('absolute top-[22px] h-2.5 w-px -translate-x-1/2', s <= (current ?? -1) ? 'bg-accent' : 'bg-line-strong')} style={{ left: pct(s, track) }} />)}
        {track.marks.map((m) => (
          <span key={`${m.kind}-${m.seq}`} title={`Step ${m.seq}: ${m.label}`} className={cn('absolute top-0.5 -translate-x-1/2 font-mono text-xs leading-4', MARK[m.kind].cls)} style={{ left: pct(m.seq, track) }}>{MARK[m.kind].glyph}</span>
        ))}
        {current != null && <div className="absolute top-[20px] size-3.5 -translate-x-1/2 rounded-full border-2 border-surface bg-accent shadow-panel" style={{ left: pct(current, track) }} />}
      </div>
    </div>
  );
}

function Legend({ track }: { track: Track }) {
  const kinds = [...new Set(track.marks.map((m) => m.kind))];
  if (!kinds.length) return null;
  return (
    <span className="hidden items-center gap-2.5 text-2xs text-muted md:flex">
      {kinds.map((k) => <span key={k} className="flex items-center gap-1"><span className={cn('font-mono', MARK[k].cls)}>{MARK[k].glyph}</span>{MARK[k].word}</span>)}
      <span className="flex items-center gap-1"><Kbd>←</Kbd><Kbd>→</Kbd> step</span>
    </span>
  );
}

const HAPPENING: Record<Happening['kind'], string> = { disruption: 'text-warn font-semibold', refusal: 'text-alarm font-semibold', error: 'text-alarm font-semibold', write: 'text-muted' };
const HAPPENING_GLYPH: Record<Happening['kind'], string> = { disruption: '▼', refusal: '▲', error: '✕', write: '■' };

/** What happened at this step and what it changed in the twin. */
function AtStep({ happened: all, changes }: { happened: Happening[]; changes: ItemChange[] }) {
  const happened = all.filter((h) => h.kind !== 'write');
  if (!happened.length && !changes.length) return <p className="px-3 pb-2 text-xs text-faint">Nothing changed in the twin at this step.</p>;
  return (
    <ul className="flex flex-wrap gap-x-4 gap-y-1 px-3 pb-2 text-xs">
      {happened.map((h) => <li key={`h${h.seq}${h.kind}`} className={HAPPENING[h.kind]}><span className="font-mono">{HAPPENING_GLYPH[h.kind]}</span> {h.text}</li>)}
      {changes.map((c) => <li key={`c${c.id}`} className={cn('font-medium', c.abnormal ? 'text-alarm' : 'text-accent')}>▸ {c.line}</li>)}
    </ul>
  );
}

export interface ScrubberProps {
  track: Track;
  current: number | null;
  index: number;
  headline: string;
  happened: Happening[];
  changes: ItemChange[];
  playing: boolean;
  onPlay: () => void;
  onStep: (seq: number) => void;
  onMove: (dir: 'prev' | 'next') => void;
  rewind: Rewind;
}

function Controls(p: ScrubberProps) {
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 px-3 pt-2">
      <div className="flex items-center gap-1">
        <Button size="icon" variant="primary" onClick={p.onPlay} aria-label={p.playing ? 'Pause' : 'Play'} title={p.playing ? 'Pause (space)' : 'Play through the steps (space)'}>{p.playing ? <Pause /> : <Play />}</Button>
        <Button size="icon" onClick={() => p.onMove('prev')} aria-label="Previous step" title="Previous view (←)"><ChevronLeft /></Button>
        <Button size="icon" onClick={() => p.onMove('next')} aria-label="Next step" title="Next view (→)"><ChevronRight /></Button>
      </div>
      <div className="flex min-w-0 flex-col">
        <span className="truncate text-sm font-semibold">{p.headline}</span>
        <span className="font-mono text-2xs text-muted">view {p.index + 1} of {p.track.stops.length}</span>
      </div>
      <span className="flex-1" />
      <Legend track={p.track} />
      {p.rewind.href
        ? <Button size="md" variant="default" asChild title={p.rewind.why}><Link to={p.rewind.href}><Undo2 />Rewind here</Link></Button>
        : <Button size="md" disabled title={p.rewind.why}><Undo2 />Rewind here</Button>}
    </div>
  );
}

/** The scrubber over the branch's steps: play and pause, step through the views, rewind from the one on screen. */
export default function Scrubber(p: ScrubberProps) {
  return (
    <div className="flex flex-col gap-1">
      <Controls {...p} />
      <TrackBar track={p.track} current={p.current} onStep={p.onStep} />
      <AtStep happened={p.happened} changes={p.changes} />
    </div>
  );
}
