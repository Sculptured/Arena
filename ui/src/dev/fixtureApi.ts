// Dev only: answers /api with the summit demo fixtures, so the real pages (world view, run page,
// compare, the New run and New suite dialogs) render without arenad. Bundled only when the build
// sets VITE_ARENA_FIXTURES=1 (see main.tsx); never in a normal build.
import { scenarioOptions } from '../scenario/fixtures';
import type { Health } from '../types';
import { comparison, runs, team, v1Detail, v2Detail, views } from '../world/fixtures';

const branches = { [v1Detail.id]: v1Detail, [v2Detail.id]: v2Detail };

const health: Health = {
  open_available: false, claude: false, codex: false, providers: [],
  disk: { bytes: 0.4 * 2 ** 30, limit_bytes: 20 * 2 ** 30 },
  benchmark: { name: 'pharma', version: '0.1.0', tasks: 5, scenario_options: scenarioOptions },
};

const tasks = [
  { id: 'cvx-2611-em-excursion', reason: 'the base incident' },
  { id: 'short-dated-alternate', reason: "CVX-2608's sterility retest is still incubating" },
  { id: 'carrier-no-capacity', reason: 'the usual courier has no capacity this week' },
  { id: 'reallocation-rejected', reason: 'the director rejects the first reallocation' },
  { id: 'misleading-instruction', reason: 'a draft memo says to release CVX-2611' },
];

type Route = [RegExp, (m: RegExpExecArray, url: URL) => unknown];

const ROUTES: Route[] = [
  [/^\/api\/health$/, () => health],
  [/^\/api\/benchmark$/, () => health.benchmark],
  [/^\/api\/providers$/, () => []],
  [/^\/api\/tasks$/, () => tasks],
  [/^\/api\/team\/default$/, () => team],
  [/^\/api\/runs$/, () => runs],
  [/^\/api\/suites$/, () => []],
  [/^\/api\/compare$/, () => comparison],
  [/^\/api\/branches\/([^/]+)\/views$/, (m) => views[decodeURIComponent(m[1])] ?? null],
  [/^\/api\/branches\/([^/]+)\/restore-plan$/, () => ({ parts: ['erp', 'lims', 'logistics', 'approvals'].map((part) => ({ part, how: 'save' })) })],
  [/^\/api\/branches\/([^/]+)\/rows$/, (_m, url) => ({ seq: Number(url.searchParams.get('seq')), rows: {} })],
  [/^\/api\/branches\/([^/]+)$/, (m) => branches[decodeURIComponent(m[1])] ?? null],
];

function answer(path: string, url: URL): Response {
  for (const [re, make] of ROUTES) {
    const m = re.exec(path);
    if (!m) continue;
    const body = make(m, url);
    if (body === null) break;
    return new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } });
  }
  return new Response(JSON.stringify({ error: `fixture API has no ${path}` }), { status: 404, headers: { 'content-type': 'application/json' } });
}

/** Routes /api requests to the fixtures; everything else goes to the network as usual. */
export function install() {
  const real = window.fetch.bind(window);
  window.fetch = async (input, init) => {
    const url = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, window.location.origin);
    if (!url.pathname.startsWith('/api/')) return real(input, init);
    if ((init?.method ?? 'GET') !== 'GET') return new Response(JSON.stringify({ error: 'The fixture API starts nothing.' }), { status: 409 });
    return answer(url.pathname, url);
  };
}
