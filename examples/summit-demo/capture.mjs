// The runbook's click path through the UI, with a screenshot of each beat at 1440x900.
//   NODE_PATH=<a node_modules with playwright> node capture.mjs OUT WORLD_URL V1_SUITE V2_SUITE
// Needs the demo arenad (reset.sh) and the driver watching (python3 drive.py watch): it drives the
// run this script starts from the New run dialog and the fork it makes with Rewind here.
import { createRequire } from 'node:module';
import { mkdirSync } from 'node:fs';
const { chromium } = createRequire(import.meta.url)('playwright');
const [out, worldUrl, v1Suite, v2Suite] = process.argv.slice(2);
const base = new URL(worldUrl).origin;
mkdirSync(out, { recursive: true });

const api = async (path) => (await fetch(base + path)).json();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function finished(branch, timeoutS = 600) {
  for (let i = 0; i < timeoutS; i++) {
    const d = await api(`/api/branches/${branch}`);
    if (d.status !== 'running') return d;
    await sleep(1000);
  }
  throw new Error(`${branch} still running`);
}
const ids = (url) => { const m = /\/(?:runs|world)\/([0-9a-f]+)\/([0-9a-f]+)/.exec(url); return { run: m[1], branch: m[2] }; };

const browser = await chromium.launch();
const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 } })).newPage();
page.setDefaultTimeout(240000);
const shot = async (name, wait = 2000) => { await page.waitForTimeout(wait); await page.screenshot({ path: `${out}/${name}.png` }); console.log('shot', name); };

// 1. World: Larchmont as every run starts.
await page.goto(worldUrl, { waitUntil: 'networkidle' });
await shot('01-world');

// 2. Disrupt: New run, an external system drives it, the courier loses capacity.
await page.goto(base + '/', { waitUntil: 'networkidle' });
await page.getByRole('button', { name: 'New run' }).first().click();
const dialog = page.getByRole('dialog');
await dialog.locator('select').nth(1).selectOption('external');
await dialog.getByText('Usual courier loses capacity').click();
await shot('02-builder', 800);
await dialog.getByRole('button', { name: 'Start run' }).click();
await page.waitForURL(/\/runs\/[0-9a-f]+\/[0-9a-f]+/);
const v1 = ids(page.url());
console.log('v1', v1);

// 3. Run: the twin changes step by step while the driver plays Team v1.
await page.getByRole('link', { name: 'World' }).first().click();
await page.waitForURL(/\/world\//);
await shot('03-run-live', 25000);
const d1 = await finished(v1.branch);

// 4. Fail: the booking Arena refused, and the violation it led to.
const refusal = d1.events.find((e) => e.kind === 'tool_call' && e.payload.name === 'book_shipment' && e.payload.error);
await page.goto(`${base}/world/${v1.run}/${v1.branch}?step=${refusal.seq}`, { waitUntil: 'networkidle' });
await shot('04-fail');
const booked = d1.events.find((e) => e.seq > refusal.seq && e.kind === 'tool_call' && e.payload.name === 'book_shipment' && !e.payload.error);
await page.goto(`${base}/world/${v1.run}/${v1.branch}?step=${booked.seq + 1}`, { waitUntil: 'networkidle' });
await shot('04b-violation');

// 5. Rewind: back to the view before the refusal, Rewind here, Swap architecture to Team v2.
await page.goto(`${base}/world/${v1.run}/${v1.branch}?step=${refusal.seq - 1}`, { waitUntil: 'networkidle' });
await shot('05-before-the-refusal');
await page.getByRole('link', { name: 'Rewind here' }).click();
await page.waitForURL(/fork=1/);
await shot('06-rewind-swap');
await page.getByRole('button', { name: /Change and rerun from here/ }).click();
await page.waitForURL((u) => !u.href.includes(v1.branch) && /\/runs\//.test(u.href), { timeout: 240000 });
const v2 = ids(page.url());
console.log('v2', v2);

// 6. Change: the rerun from the fork, Team v2 driven from that point.
await page.getByRole('link', { name: 'World' }).first().click();
await shot('07-rerun-live', 12000);
const d2 = await finished(v2.branch);
const v2booked = d2.events.find((e) => e.seq >= d2.start_seq && e.kind === 'tool_call' && e.payload.name === 'book_shipment' && !e.payload.error);
await page.goto(`${base}/world/${v2.run}/${v2.branch}?step=${v2booked.seq + 1}`, { waitUntil: 'networkidle' });
await shot('08-rerun-world');

// 7. Compare: outcomes, not transcripts.
await page.goto(`${base}/compare/${v1.branch}/${v2.branch}`, { waitUntil: 'networkidle' });
await shot('09-compare', 6000);

// 8. Batch: the same comparison as rates over the seeded incidents.
await page.goto(`${base}/suites/compare/${v1Suite}/${v2Suite}`, { waitUntil: 'networkidle' });
await shot('10-batch', 4000);
console.log(JSON.stringify({ v1, v2, d1: d1.status, d2: d2.status }));
await browser.close();
