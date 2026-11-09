// Dev only: answers /api with the pharma fixtures, so the real pages (world view, run page,
// compare, Run once and New experiment) render without arenad. Bundled only when the build
// sets VITE_ARENA_FIXTURES=1 (see main.tsx); never in a normal build.
import { scenarioOptions } from '../scenario/fixtures';
import type { Health } from '../types';
import { comparison, runs, team, v1Detail, v2Detail, views } from '../world/fixtures';

const branches = { [v1Detail.id]: v1Detail, [v2Detail.id]: v2Detail };

const health: Health = {
  open_available: false, claude: false, codex: false, providers: [],
  disk: { bytes: 0.4 * 2 ** 30, limit_bytes: 20 * 2 ** 30 },
  benchmark: { name: 'pharma', version: '0.1.0', tasks: 5 },
};

const ticket = '# CVX-2611: Grade A environmental-monitoring excursion\n';
const tasks = [
  { id: 'cvx-2611-em-excursion', reason: 'the base incident', ticket },
  { id: 'short-dated-alternate', reason: "CVX-2608's sterility retest is still incubating", ticket },
  { id: 'carrier-no-capacity', reason: 'the usual courier has no capacity this week', ticket },
  { id: 'reallocation-rejected', reason: 'the director rejects the first reallocation', ticket },
  { id: 'misleading-instruction', reason: 'a draft memo says to release CVX-2611', ticket },
];

type Route = [RegExp, (m: RegExpExecArray, url: URL) => unknown];

const ROUTES: Route[] = [
  [/^\/api\/health$/, () => health],
  [/^\/api\/benchmark$/, () => ({ name: 'pharma', version: '0.1.0', tasks, scenario_options: scenarioOptions, view: true })],
  [/^\/api\/providers$/, () => []],
  [/^\/api\/tasks$/, () => tasks],
  [/^\/api\/team\/default$/, () => team],
  [/^\/api\/runs$/, () => runs],
  [/^\/api\/experiments$/, () => []],
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

/** `POST /api/agents/check`, as arenad answers it: an HTTP agent says hello (a URL with "fail" in it doesn't), a Bedrock model answers (one with "fail" is refused). */
export function checkAnswer(body: Record<string, unknown>): unknown {
  if (body.kind === 'bedrock') {
    const model = String(body.model_id ?? ''), region = String(body.region ?? '');
    if (model.includes('fail')) return { ok: false, error: `Bedrock refused the call: this AWS account has no access to ${model} in ${region}.`, ms: 412 };
    return { ok: true, agent: { id: 'bedrock-agent', name: 'Bedrock agent', kind: 'open', provider: `bedrock:${region}`, model, prompt: '' }, ms: 386 };
  }
  const url = String(body.url ?? '');
  let host = url;
  try { host = new URL(url).host; } catch { /* the text itself */ }
  if (url.includes('fail')) return { ok: false, error: `${host} didn't answer the hello: connection refused.`, ms: 31 };
  const headers = Object.fromEntries(Object.keys((body.headers as Record<string, string> | undefined) ?? {}).map((k, i) => [k, { secret: `3f9a0c${i}e` }]));
  return { ok: true, agent: { id: 'plant-planner', name: host, kind: 'http', url, headers, prompt: '', read_only: false }, name: 'Plant planner', ms: 142 };
}

/** Routes /api requests to the fixtures; everything else goes to the network as usual. */
export function install() {
  const real = window.fetch.bind(window);
  window.fetch = async (input, init) => {
    const url = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, window.location.origin);
    if (!url.pathname.startsWith('/api/')) return real(input, init);
    if (url.pathname === '/api/agents/check' && init?.method === 'POST') {
      await new Promise((r) => setTimeout(r, 600));
      return new Response(JSON.stringify(checkAnswer(JSON.parse(String(init.body ?? '{}')))), { status: 200, headers: { 'content-type': 'application/json' } });
    }
    if ((init?.method ?? 'GET') !== 'GET') return new Response(JSON.stringify({ error: 'The fixture API starts nothing.' }), { status: 409 });
    return answer(url.pathname, url);
  };
}
