import { Sparkles, Undo2 } from 'lucide-react';
import type { ReactNode } from 'react';
import { Button } from '../components/ui/button';
import { Tag } from '../components/ui/status';
import type { BranchDetail, Change } from '../types';
import { KIND_LABEL, SEVERITY_LABEL, canTry, prefillOf, smallPrint, tryStep } from './present';
import { useReviewEnabled } from './setting';
import type { RunReview, StoredReview, Suggestion, WentWrong } from './types';
import { useReview } from './useReview';
import { ReviewFrame } from './ReviewFrame';

/** A step a review cites: a small link that selects it. */
export function StepChip({ seq, onStep, label }: { seq: number | string; onStep?: () => void; label?: string }) {
  const text = label ?? `Step ${seq}`;
  if (!onStep) return <span className="font-mono text-2xs text-muted">{text}</span>;
  return (
    <button type="button" onClick={onStep} className="rounded-sm border border-line bg-sunken px-1.5 font-mono text-2xs text-muted hover:border-line-strong hover:text-fg">
      {text}
    </button>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex min-w-0 flex-col gap-2">
      <h3 className="text-2xs font-semibold tracking-wide text-muted uppercase">{title}</h3>
      {children}
    </section>
  );
}

const severityTone = (s: WentWrong['severity']) => (s === 'safety' ? 'alarm' : s === 'goal' ? 'warn' : undefined);

function Wrong({ w, onStep }: { w: WentWrong; onStep: (seq: number) => void }) {
  return (
    <li className="flex flex-col gap-1">
      <div className="flex flex-wrap items-center gap-1.5">
        <Tag tone={severityTone(w.severity)}>{SEVERITY_LABEL[w.severity] ?? w.severity}</Tag>
        <span className="text-sm font-medium">{w.title}</span>
      </div>
      <p className="text-sm leading-5 text-muted">{w.why}</p>
      <div className="flex flex-wrap gap-1">
        {w.steps.map((s) => <StepChip key={s} seq={s} onStep={() => onStep(s)} />)}
        {w.unverified && <span className="text-2xs text-faint">its steps weren't in the run</span>}
      </div>
    </li>
  );
}

function Tip({ s, d, onTry }: { s: Suggestion; d: BranchDetail; onTry?: (seq: number, prefill: Record<string, Change>) => void }) {
  const agent = s.agent ? d.team.agents.find((a) => a.id === s.agent)?.name ?? s.agent : null;
  const prefill = prefillOf(s, d.team);
  const seq = canTry(s) ? tryStep(s, d.events) : null;
  return (
    <li className="flex flex-col gap-1">
      <div className="flex flex-wrap items-center gap-1.5">
        <Tag>{KIND_LABEL[s.kind] ?? s.kind}</Tag>{agent && <span className="text-2xs text-muted">{agent}</span>}
      </div>
      <p className="text-sm font-medium leading-5">{s.change}</p>
      <p className="text-sm leading-5 text-muted">{s.why}</p>
      {s.prompt_text && <blockquote className="border-l-2 border-line-strong pl-2 text-xs text-muted">{s.prompt_text}</blockquote>}
      {onTry && prefill && seq != null && (
        <Button size="sm" className="self-start" onClick={() => onTry(seq, prefill)} title={`Open "Change and rerun" at step ${seq} with this added to the prompt`}>
          <Undo2 />Rewind and try this
        </Button>
      )}
    </li>
  );
}

export function RunReviewBody({ r, d, onStep, onTry }: { r: RunReview; d: BranchDetail; onStep: (seq: number) => void; onTry?: (seq: number, prefill: Record<string, Change>) => void }) {
  return (
    <div className="flex flex-col gap-4 px-3 py-3">
      <p className="text-base leading-6 font-medium">{r.verdict}</p>
      <div className="grid gap-5 lg:grid-cols-2">
        {r.went_wrong.length > 0 && <Section title="Where it went wrong"><ul className="flex flex-col gap-3">{r.went_wrong.map((w, i) => <Wrong key={i} w={w} onStep={onStep} />)}</ul></Section>}
        {r.suggestions.length > 0 && <Section title="Suggestions"><ul className="flex flex-col gap-3">{r.suggestions.map((s, i) => <Tip key={i} s={s} d={d} onTry={onTry} />)}</ul></Section>}
      </div>
      {r.went_well.length > 0 && (
        <Section title="What went well">
          <ul className="flex flex-col gap-0.5 text-sm text-muted">{r.went_well.map((w, i) => <li key={i}>✓ {w}</li>)}</ul>
        </Section>
      )}
    </div>
  );
}

/** The small print, and the citations arenad dropped. */
function Footer({ s }: { s: StoredReview }) {
  const dropped = s.issues.filter((i) => i.startsWith('dropped citation')).length;
  return (
    <p className="border-t border-line px-3 py-1.5 text-2xs text-faint">
      {smallPrint(s)}{s.review && 'confidence' in s.review ? ` · confidence ${s.review.confidence}` : ''}
      {dropped > 0 && ` · ${dropped} citation${dropped === 1 ? '' : 's'} to steps not in the run left out`}
    </p>
  );
}

/** Before any review: what it is, and the button. */
function Offer({ onStart }: { onStart: () => void }) {
  return (
    <div className="flex flex-wrap items-center gap-3 px-3 py-2.5">
      <span className="text-sm text-muted">A model can read this run's record and say where it went wrong and what to change.</span>
      <Button size="sm" variant="primary" onClick={onStart}><Sparkles />Review this run</Button>
    </div>
  );
}

/**
 * The AI review of an ended run: the verdict, where it went wrong (each step selects it),
 * suggestions (prompt and team changes can be tried from the step they name) and what went well.
 * Hidden when AI review is off. With `auto`, a run without a review is reviewed at once.
 */
export default function ReviewCard({ d, auto, onStep, onTry }: {
  d: BranchDetail;
  auto: boolean;
  onStep: (seq: number) => void;
  onTry?: (seq: number, prefill: Record<string, Change>) => void;
}) {
  const enabled = useReviewEnabled();
  const ended = d.status !== 'running';
  const h = useReview<RunReview>(ended ? { kind: 'run', branch: d.id } : null, enabled && ended, auto);
  if (!enabled || !ended || !h.loaded) return null;
  return (
    <ReviewFrame title="Review" label="AI review" h={h} what="the run's record" retryHint="Nothing else changed. Try again, or turn AI review off under Advanced."
      empty={<Offer onStart={() => h.start()} />} done={(r) => <RunReviewBody r={r} d={d} onStep={onStep} onTry={onTry} />} footer={(s) => <Footer s={s} />} />
  );
}
