import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { Tag } from '../components/ui/status';
import { suite as fetchSuite } from '../control/api';
import type { Target } from './api';
import { ReviewFrame } from './ReviewFrame';
import { smallPrint } from './present';
import { useReviewEnabled } from './setting';
import type { CompareReview, Difference } from './types';
import { useReview } from './useReview';

type Side = 'a' | 'b';
/** Where a citation of one side leads (a step of a run, or a run of a suite); null: nowhere. */
export type HrefOf = (side: Side, cite: number | string) => string | null;

const BETTER = { a: 'A did better', b: 'B did better', mixed: 'Mixed' } as const;

/** A citation: "Step 12" for a run, "Run 1a2b3c4d" for a suite's example run. */
export function citeLabel(cite: number | string): string {
  return typeof cite === 'number' ? `Step ${cite}` : `Run ${cite.slice(0, 8)}`;
}

function Cites({ side, cites, hrefOf }: { side: Side; cites: (number | string)[]; hrefOf: HrefOf }) {
  return (
    <span className="flex flex-wrap gap-1">
      {cites.map((c) => {
        const href = hrefOf(side, c);
        const cls = 'rounded-sm border border-line bg-sunken px-1.5 font-mono text-2xs text-muted';
        return href ? <Link key={String(c)} to={href} className={`${cls} hover:border-line-strong hover:text-fg`}>{citeLabel(c)}</Link> : <span key={String(c)} className={cls}>{citeLabel(c)}</span>;
      })}
    </span>
  );
}

function Diff({ d, hrefOf }: { d: Difference; hrefOf: HrefOf }) {
  const side = (label: string, s: Side, text: string, cites: (number | string)[]) => (
    <div className="flex min-w-0 flex-col gap-1 rounded-md border border-line bg-raised px-2.5 py-2">
      <span className="font-mono text-2xs text-faint">{label}</span>
      <p className="text-sm leading-5">{text || '–'}</p>
      <Cites side={s} cites={cites} hrefOf={hrefOf} />
    </div>
  );
  return (
    <li className="flex flex-col gap-1.5">
      <span className="text-sm font-medium">{d.title}</span>
      <div className="grid gap-2 sm:grid-cols-2">{side('A', 'a', d.a, d.steps_a)}{side('B', 'b', d.b, d.steps_b)}</div>
      <p className="text-sm leading-5 text-muted">{d.why_it_matters}</p>
    </li>
  );
}

export function CompareBody({ r, hrefOf }: { r: CompareReview; hrefOf: HrefOf }) {
  return (
    <div className="flex flex-col gap-3 px-4 pb-3">
      <div className="flex flex-wrap items-baseline gap-2">
        <Tag>{BETTER[r.which_is_better] ?? r.which_is_better}</Tag>
        <p className="min-w-0 flex-1 text-base leading-6 font-medium">{r.headline}</p>
      </div>
      <ol className="flex flex-col gap-4">{r.differences.map((d, i) => <Diff key={i} d={d} hrefOf={hrefOf} />)}</ol>
      {r.why && <p className="text-sm text-muted">{r.why}</p>}
    </div>
  );
}

/**
 * "What matters" between two runs or two suites: a headline and the few differences that matter,
 * each with links to the steps (or runs) of A and B. Reviewed as soon as it shows; hidden when AI
 * review is off.
 */
export default function WhatMatters({ target, hrefOf, what }: { target: Target; hrefOf: HrefOf; what: string }) {
  const enabled = useReviewEnabled();
  const h = useReview<CompareReview>(target, enabled, true);
  if (!enabled || !h.loaded) return null;
  return (
    <ReviewFrame title="What matters" label="What matters" h={h} what={what} retryHint="The comparison below is unchanged."
      headline={(r) => r.headline} done={(r) => <CompareBody r={r} hrefOf={hrefOf} />} footer={(s) => <p className="px-4 pb-3 text-2xs text-faint">{smallPrint(s)}</p>} />
  );
}

/** What matters between two suites: example runs link to their run pages. */
export function SuiteWhatMatters({ a, b }: { a: string; b: string }) {
  const [runOf, setRunOf] = useState<Record<string, string>>({});
  useEffect(() => {
    Promise.all([fetchSuite(a), fetchSuite(b)]).then((rs) => {
      const m: Record<string, string> = {};
      for (const r of rs) for (const row of r.runs) if (row.branch_id && row.run_id) m[row.branch_id] = row.run_id;
      setRunOf(m);
    }, () => {});
  }, [a, b]);
  const hrefOf: HrefOf = (_side, cite) => (typeof cite === 'string' && runOf[cite] ? `/runs/${runOf[cite]}/${cite}` : null);
  return <WhatMatters target={{ kind: 'suites', a, b }} hrefOf={hrefOf} what="both suites' results" />;
}
