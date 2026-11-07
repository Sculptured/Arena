import { useCallback, useMemo } from 'react';
import { Panel } from '../components/ui/panel';
import { useStepParam } from '../run/useStepParam';
import type { BranchDetail } from '../types';
import KpiStrip from './KpiStrip';
import { worldState } from './page';
import { moveStop } from './scrub';
import Scrubber from './Scrubber';
import TwinCanvas from './TwinCanvas';
import type { BranchView } from './types';
import { usePlay, useStopKeys } from './useWorld';

/** The world view's state for the step in the address (the latest view when none), and the scrubber's controls. */
export function useWorldView(runId: string, d: BranchDetail, views: BranchView[]) {
  const [step, setStep] = useStepParam();
  const s = useMemo(() => worldState({ views, step, events: d.events, team: d.team, startSeq: d.start_seq, runId, branchId: d.id, running: d.status === 'running' }),
    [views, step, d, runId]);
  const stops = s.track.stops;
  const play = usePlay(stops, step, setStep);
  const { pause } = play;
  const move = useCallback((dir: 'prev' | 'next') => { pause(); setStep((cur) => moveStop(stops, cur, dir)); }, [pause, setStep, stops]);
  useStopKeys(stops, setStep, play.toggle, pause);
  return { s, step, setStep, play, move };
}

/** The twin at the step on screen: its KPIs, the scrubber (when `scrub`), and the overview. */
export default function TwinView({ w, views, scrub = true }: { w: ReturnType<typeof useWorldView>; views: BranchView[]; scrub?: boolean }) {
  const { s, setStep, play, move } = w;
  const all = useMemo(() => views.map((v) => v.view), [views]);
  if (!s.view) return null;
  return (
    <div className="flex flex-col gap-2.5">
      <KpiStrip kpis={s.view.kpis} before={s.changes.kpis} clock={s.view.clock} />
      {scrub && (
        <Panel>
          <Scrubber track={s.track} current={s.current} index={s.index} headline={s.headline} happened={s.happened} changes={s.changes.items}
            playing={play.playing} onPlay={play.toggle} onStep={(seq) => { play.pause(); setStep(seq); }} onMove={move} rewind={s.rewind} />
        </Panel>
      )}
      <Panel className="p-2.5"><TwinCanvas view={s.view} changes={s.changes} all={all} /></Panel>
    </div>
  );
}
