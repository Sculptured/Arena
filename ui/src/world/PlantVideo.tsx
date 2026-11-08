import { useEffect, useRef, useState } from 'react';
import { Panel } from '../components/ui/panel';
import type { Event } from '../types';
import { type Recording, arenaSecondAt, clockText, recording } from './recording';

/** "Plant (Isaac Sim)": the branch's recorded plant, held at the arena second of the step on screen. Nothing when the
 *  branch has no recording. Its own controls play on from there. */
export default function PlantVideo({ branchId, events, step }: { branchId: string; events: Event[]; step: number | null }) {
  const [rec, setRec] = useState<Recording | null>(null);
  const ref = useRef<HTMLVideoElement>(null);
  useEffect(() => {
    let live = true;
    recording(branchId).then((r) => live && setRec(r)).catch(() => live && setRec(null));
    return () => { live = false; };
  }, [branchId]);
  const t = arenaSecondAt(events, step);
  useEffect(() => {
    const v = ref.current;
    if (!v || t == null) return;
    const seek = () => { v.pause(); v.currentTime = Math.min(t, Number.isFinite(v.duration) ? v.duration : t); };
    if (v.readyState >= 1) seek(); else v.addEventListener('loadedmetadata', seek, { once: true });
  }, [t, rec]);
  if (!rec?.video) return null;
  const inherited = rec.of && rec.of !== branchId ? ' (recorded on the branch it was forked from)' : '';
  return (
    <Panel className="p-2.5" aria-label="Plant (Isaac Sim)">
      <div className="mb-1.5 flex items-baseline justify-between text-xs text-muted-foreground">
        <span className="font-medium text-foreground">Plant (Isaac Sim)</span>
        <span>{t == null ? 'arena time unknown' : `arena ${clockText(t)}`}{inherited}</span>
      </div>
      <video ref={ref} src={rec.video} controls muted playsInline preload="metadata" className="w-full rounded bg-black" />
    </Panel>
  );
}
