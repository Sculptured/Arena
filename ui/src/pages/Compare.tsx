import { useEffect, useState } from 'react';
import { Link, useParams } from 'react-router-dom';
import { branch as fetchBranch, compare, views as fetchViews } from '../api';
import OutcomePanel from '../compare/OutcomePanel';
import { normalizeViews } from '../world/twin';
import type { BranchView, TwinView } from '../world/types';
import { ApproximateMark, StatusChip } from '../components';
import { PageBody, PageHeader } from '../components/ui/page';
import { Panel, PanelHeader } from '../components/ui/panel';
import { ErrorState, LoadingState, Notice } from '../components/ui/state';
import { Tag } from '../components/ui/status';
import { cn } from '@/lib/cn';
import { agentModelLine, costIsUnknown, teamProviders } from '../providers';
import { APPROXIMATE_NOTE, isApproximate, notComparedLine } from '../restore';
import { pretty } from '../run/Inspector';
import { diffCounts, formatCost, jsonDiff } from '../run/timeline';
import type { BranchDetail, CompareResponse, CompareSide, Event, RowDiff, Team } from '../types';

function StepView({ label, id, run, ev }: { label: string; id: string; run?: string; ev: Event | undefined }) {
  const p = (ev?.payload ?? {}) as Record<string, unknown>;
  return (
    <Panel>
      <PanelHeader title={`${label} at this step`} meta={<span className="font-mono">{id.slice(0, 8)}</span>}>
        {ev && run && <Link className="text-xs text-accent hover:underline" to={`/runs/${run}/${id}?step=${ev.seq}`}>Open step</Link>}
      </PanelHeader>
      {!ev ? <p className="p-3 text-sm text-muted">No step: this branch had ended.</p> : (
        <div className="flex flex-col gap-2 p-3">
          <div className="flex flex-wrap items-center gap-1.5 text-xs">
            <span className="font-mono font-semibold">step {ev.seq}</span><span className="text-muted">{ev.agent ?? 'run'}</span>
            <Tag>{ev.kind.replace('_', ' ')}</Tag>{typeof p.name === 'string' && <span className="font-mono">{p.name}</span>}
          </div>
          <pre className="max-h-64 overflow-auto rounded-md border border-line bg-sunken p-2 font-mono text-2xs whitespace-pre-wrap break-all">{pretty(ev.kind === 'tool_call' ? p.args : p)}</pre>
        </div>
      )}
    </Panel>
  );
}

/** One branch's outcome, numbers and agents (each with its model and where it runs). */
export function Side({ id, s, team, label = '' }: { id: string; s: CompareSide; team?: Team; label?: string }) {
  const unknown = teamProviders(team).some(costIsUnknown);
  const row = (k: string, v: React.ReactNode) => <><dt className="text-2xs font-medium tracking-wide text-muted uppercase">{k}</dt><dd>{v}</dd></>;
  return (
    <Panel>
      <PanelHeader title={label || 'Branch'} meta={<span className="font-mono">{id.slice(0, 8)}</span>}><StatusChip status={s.status} /><ApproximateMark of={s} /></PanelHeader>
      <dl className="grid grid-cols-[max-content_1fr] items-baseline gap-x-4 gap-y-1.5 p-3 text-sm">
        {row('Seconds', <span className="font-mono">{s.seconds != null ? s.seconds.toFixed(1) : '–'}</span>)}
        {row('Tokens', <span className="font-mono">{s.tokens?.toLocaleString() ?? '–'}</span>)}
        {row('Cost', formatCost(s.cost_usd, s.cost_source, unknown))}
        {team && row('Agents', <ul className="flex flex-col gap-0.5 text-xs">{team.agents.map((a) => <li key={a.id}>{a.name}: <span className="font-mono text-2xs text-muted">{agentModelLine(a)}</span></li>)}</ul>)}
      </dl>
    </Panel>
  );
}

function RowDoc({ r, open }: { r: RowDiff; open: boolean }) {
  const d = jsonDiff(r.a ?? undefined, r.b ?? undefined);
  const n = diffCounts(d);
  return (
    <details open={open} className="group border-t border-line px-3 py-1.5">
      <summary className="flex cursor-pointer items-center gap-2 py-0.5 font-mono text-xs marker:text-faint">
        <span className="min-w-0 truncate">{r.key}</span>{!r.a && <Tag>only in B</Tag>}{!r.b && <Tag>only in A</Tag>}
        <span className="ml-auto shrink-0 text-2xs text-muted">−{n.removed} +{n.added}</span>
      </summary>
      <pre className="mt-1 mb-1 max-h-72 overflow-auto rounded-md border border-line bg-sunken py-1 font-mono text-2xs leading-4">
        {d.map((l, i) => (
          <div key={i} className={cn('px-2 whitespace-pre-wrap break-all', l.op === '+' && 'bg-accent-soft font-semibold', l.op === '-' && 'text-muted line-through decoration-faint')}>
            <span aria-hidden="true" className="inline-block w-3 text-faint">{l.op === '-' ? '−' : l.op}</span>{l.text}
          </div>
        ))}
      </pre>
    </details>
  );
}

/** How many differing rows start open; the rest open on a click. */
const OPEN_ROWS = 3;

/** The final states' differing rows, and the parts left out (never read as "identical"). */
export function Rows({ data }: { data: CompareResponse }) {
  const skipped = notComparedLine(data.not_compared, 'compare');
  return (
    <Panel>
      <PanelHeader title="Differing rows" meta={`${data.differing_rows.length} in the final worlds · − A, + B`} />
      {skipped && <p className="px-3 pt-2 text-xs">{skipped}</p>}
      {data.differing_rows.length === 0 && <p className="p-3 text-sm text-muted">{skipped ? 'No differences in the parts compared.' : 'The final worlds are identical.'}</p>}
      {data.differing_rows.map((r, i) => <RowDoc key={r.key} r={r} open={i < OPEN_ROWS} />)}
    </Panel>
  );
}

interface Loaded {
  data: CompareResponse;
  a: BranchDetail;
  b: BranchDetail;
  /** Each branch's twin at the end, from the views endpoint; undefined when it can't be read (the events' own views are used). */
  va?: TwinView | null;
  vb?: TwinView | null;
}

/** The branch's last view from `GET /api/branches/{id}/views`; undefined when the endpoint fails. */
const endView = (id: string) => fetchViews(id).then((v) => v, () => undefined);

const lastOf = (raw: BranchView[] | undefined): TwinView | null | undefined =>
  raw === undefined ? undefined : normalizeViews(raw).at(-1)?.view ?? null;

function useCompare(a: string, b: string) {
  const [got, setGot] = useState<Loaded | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let cancelled = false;
    setGot(null); setError(null);
    Promise.all([compare(a, b), fetchBranch(a), fetchBranch(b), endView(a), endView(b)])
      .then(([data, x, y, rx, ry]) => { if (!cancelled) setGot({ data, a: x, b: y, va: lastOf(rx), vb: lastOf(ry) }); })
      .catch((e) => { if (!cancelled) setError(String(e.message ?? e)); });
    return () => { cancelled = true; };
  }, [a, b, attempt]);
  return { got, error, retry: () => setAttempt((n) => n + 1) };
}

function Divergence({ got, a, b }: { got: Loaded; a: string; b: string }) {
  const seq = got.data.first_divergent_seq;
  return (
    <>
      <Panel className="px-3 py-2 text-sm">
        {seq != null ? <><span className="font-semibold">First divergence at step {seq}.</span> <span className="text-muted">Everything before it is the same in both branches.</span></>
          : <span className="font-semibold">The branches never diverge.</span>}
      </Panel>
      {seq != null && (
        <div className="grid gap-3 lg:grid-cols-2">
          <StepView label="A" id={a} run={got.a.run_id} ev={got.a.events.find((e) => e.seq === seq)} />
          <StepView label="B" id={b} run={got.b.run_id} ev={got.b.events.find((e) => e.seq === seq)} />
        </div>
      )}
    </>
  );
}

export default function Compare() {
  const { a = '', b = '' } = useParams();
  const { got, error, retry } = useCompare(a, b);
  return (
    <>
      <PageHeader title="Compare" meta={<span>branch <span className="font-mono">{a.slice(0, 8)}</span> (A) against <span className="font-mono">{b.slice(0, 8)}</span> (B)</span>} />
      <PageBody>
        {error && <ErrorState error={error} onRetry={retry} hint="Both branches must have finished and kept their final saves." />}
        {!got && !error && <Panel><LoadingState label="Comparing the two final worlds… The first comparison of a pair can take up to a minute while the saves are read." rows={4} /></Panel>}
        {got && (
          <>
            <OutcomePanel a={{ detail: got.a, side: got.data.a, view: got.va }} b={{ detail: got.b, side: got.data.b, view: got.vb }} first={got.data.first_divergent_seq} />
            <div className="grid gap-3 lg:grid-cols-2"><Side label="A" id={a} s={got.data.a} team={got.a.team} /><Side label="B" id={b} s={got.data.b} team={got.b.team} /></div>
            {(isApproximate(got.data.a) || isApproximate(got.data.b)) && <Notice>{APPROXIMATE_NOTE}</Notice>}
            <Divergence got={got} a={a} b={b} />
            <Rows data={got.data} />
          </>
        )}
      </PageBody>
    </>
  );
}
