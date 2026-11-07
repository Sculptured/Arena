// Records the summit video's scenes from the real Arena UI, one clip per narrated scene.
//   NODE_PATH=<node_modules with playwright> node record.mjs [scene ...]     (all scenes by default)
// Needs the demo arenad and the driver watching (python3 drive.py watch): `disrupt` starts Team v1's
// live run from the New run dialog and `rewind` forks it to Team v2, and the scenes after each wait
// for the driver to finish them. Each scene lasts its narration (timings.json, from
// narrate_kokoro.py) plus build.py's lead-in and tail; every spotlight and click starts on the
// sentence that explains it. Frames come from Chrome's screencast at 1920x1080 (1600x900 at 1.2x).
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
const { chromium } = createRequire(import.meta.url)('playwright');

const HERE = dirname(fileURLToPath(import.meta.url));
const BASE = process.env.ARENA_DEMO_URL ?? 'http://127.0.0.1:4513';
const WORK = process.env.SUMMIT_VIDEO_WORK ?? '/home/alexsky2/sculptured-work-local/video/summit';
const MEDIA = join(WORK, 'media');
const STATE = join(WORK, 'state.json');
const NARR = Object.fromEntries(JSON.parse(readFileSync(join(WORK, 'narration/timings.json'), 'utf8')).scenes.map((s) => [s.id, s]));
const LEAD = 0.4, TAIL = 0.6; // build.py's lead-in and tail around the narration
const VIEW = { width: 1600, height: 900 }, SCALE = 1.2, FPS = 30;
const sleep = (ms) => new Promise((r) => setTimeout(r, Math.max(0, ms)));
const api = async (path, body) => (await fetch(BASE + path, body ? { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) } : undefined)).json();
const state = existsSync(STATE) ? JSON.parse(readFileSync(STATE, 'utf8')) : {};
const saveState = () => writeFileSync(STATE, JSON.stringify(state, null, 2));

// ---- capture: Chrome's screencast, frames timed by the compositor, made constant-rate by ffmpeg
async function capture(page) {
  const cdp = await page.context().newCDPSession(page);
  const dir = mkdtempSync(join(WORK, 'frames-'));
  const frames = [];
  cdp.on('Page.screencastFrame', (f) => {
    const file = join(dir, `f${String(frames.length).padStart(6, '0')}.jpg`);
    writeFileSync(file, Buffer.from(f.data, 'base64'));
    frames.push({ file, ts: f.metadata.timestamp });
    cdp.send('Page.screencastFrameAck', { sessionId: f.sessionId }).catch(() => {});
  });
  await cdp.send('Page.startScreencast', { format: 'jpeg', quality: 94, maxWidth: 1920, maxHeight: 1080, everyNthFrame: 1 });
  return {
    async stop(t0, total, out) {
      await cdp.send('Page.stopScreencast');
      await sleep(300);
      const end = t0 + total;
      const keep = frames.filter((f) => f.ts < end);
      const first = keep.findLastIndex((f) => f.ts <= t0);
      const seq = keep.slice(Math.max(0, first));
      if (!seq.length) throw new Error('no frames captured');
      const lines = seq.map((f, i) => {
        const from = Math.max(f.ts, t0), to = i + 1 < seq.length ? Math.max(seq[i + 1].ts, t0) : end;
        return `file '${f.file}'\nduration ${Math.max(0, to - from).toFixed(4)}\n`;
      });
      writeFileSync(join(dir, 'list.txt'), lines.join('') + `file '${seq.at(-1).file}'\n`);
      execFileSync('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-y', '-f', 'concat', '-safe', '0', '-i', join(dir, 'list.txt'),
        '-vf', `fps=${FPS},scale=1920:1080:flags=lanczos,format=yuv420p`, '-t', total.toFixed(3),
        '-c:v', 'libx264', '-preset', 'slow', '-crf', '14', out]);
      console.log(`  ${out}: ${seq.length} frames over ${total.toFixed(1)} s`);
      rmSync(dir, { recursive: true, force: true });
    },
  };
}

// ---- the director: cursor, clicks and spotlights over the real page
function director(page) {
  let cur = { x: VIEW.width * 0.62, y: VIEW.height * 0.55 };
  const pad = (b, p) => ({ x: b.x - p, y: b.y - p, w: b.width + 2 * p, h: b.height + 2 * p });
  async function box(targets, p = 8) {
    const boxes = [];
    for (const t of [].concat(targets)) {
      const b = await t.boundingBox();
      if (b) boxes.push(b);
    }
    if (!boxes.length) throw new Error('nothing to spotlight');
    const x0 = Math.min(...boxes.map((b) => b.x)), y0 = Math.min(...boxes.map((b) => b.y));
    const x1 = Math.max(...boxes.map((b) => b.x + b.width)), y1 = Math.max(...boxes.map((b) => b.y + b.height));
    return pad({ x: x0, y: y0, width: x1 - x0, height: y1 - y0 }, p);
  }
  const d = {
    async restore() { await page.evaluate((c) => { window.__ov.mount(); window.__ov.cursor(c.x, c.y, 0); }, cur); },
    async spot(targets, card, o = {}) {
      const r = await box(targets, o.pad ?? 8);
      await page.evaluate(([r, card, tone, pref]) => window.__ov.spot(r, card, tone, pref), [r, card ?? null, o.tone ?? '', o.place ?? '']);
    },
    async clear() { await page.evaluate(() => window.__ov.clear()); },
    async move(target, ms = 800, at = [0.5, 0.5]) {
      const p = Array.isArray(target) ? { x: target[0], y: target[1] } : await target.boundingBox().then((b) => ({ x: b.x + b.width * at[0], y: b.y + b.height * at[1] }));
      cur = p;
      await page.evaluate(([x, y, ms]) => window.__ov.cursor(x, y, ms), [p.x, p.y, ms]);
      await sleep(ms + 80);
      return p;
    },
    async click(target, ms = 800, at) {
      const p = await d.move(target, ms, at);
      await page.evaluate(([x, y]) => window.__ov.ripple(x, y), [p.x, p.y]);
      await sleep(120);
      await page.mouse.click(p.x, p.y);
    },
    async hideCursor() { await page.evaluate(() => { window.__ov.mount(); document.getElementById('ov-cursor').style.display = 'none'; }); },
  };
  return d;
}

// ---- the twin's scrubber
const stepOf = (page) => Number(new URL(page.url()).searchParams.get('step') ?? NaN);
async function stepTo(page, seq, gap = 700) {
  for (let i = 0; i < 80 && !(stepOf(page) >= seq); i++) {
    const before = page.url();
    await page.keyboard.press('ArrowRight');
    await sleep(gap);
    if (page.url() === before) break; // the last view
  }
}
async function clickBackTo(page, d, seq, gap = 650) {
  const prev = page.getByRole('button', { name: 'Previous step' });
  for (let i = 0; i < 40 && stepOf(page) > seq; i++) { await d.click(prev, i ? 160 : 700); await sleep(gap); }
}
const zone = (page, label) => page.locator('div.absolute.rounded-md.border').filter({ has: page.locator('span.uppercase', { hasText: new RegExp(`^${label}$`, 'i') }) }).first();
const tile = (page, label) => page.locator(`[title^="${label}"]`).first();
const kpi = (page, label) => page.locator('div.rounded-lg.border', { hasText: label }).first();
const mark = (page, seq) => page.locator(`span[title^="Step ${seq}:"]`).first();
const track = (page) => page.locator('div.touch-none').first();
const headline = (page) => page.locator('span.truncate.text-sm.font-semibold').first();
const atStep = (page) => track(page).locator('xpath=../following-sibling::*[1]');
const worldUrl = (r, step) => `${BASE}/world/${r.run}/${r.branch}${step != null ? `?step=${step}` : ''}`;

async function settle(page, ms = 1500) {
  await page.waitForLoadState('networkidle').catch(() => {});
  await sleep(ms);
}
async function finished(branch, timeoutS = 900) {
  for (let i = 0; i < timeoutS; i++) {
    const b = await api(`/api/branches/${branch}`);
    if (b.status && b.status !== 'running') return b;
    await sleep(1000);
  }
  throw new Error(`${branch} is still running`);
}
const toolCalls = (b) => b.events.filter((e) => e.kind === 'tool_call');
const views = async (branch) => (await api(`/api/branches/${branch}/views`)).map((v) => v.seq);
const stopBefore = (stops, seq) => [...stops].reverse().find((s) => s < seq);
const stopAfter = (stops, seq) => stops.find((s) => s >= seq);

/** Facts about Team v1's branch: the refused booking, the booking after it, the disruption. */
async function v1Facts() {
  const b = await finished(state.v1.branch);
  const calls = toolCalls(b);
  const refused = calls.find((e) => e.payload.name === 'book_shipment' && e.payload.error);
  const booked = calls.find((e) => e.seq > refused.seq && e.payload.name === 'book_shipment' && !e.payload.error);
  const disruption = b.events.find((e) => e.kind === 'world_event' && e.payload.kind === 'disruption');
  return { b, refused: refused.seq, booked: booked.seq, order: booked.payload.args.order, disruption: disruption?.seq, stops: await views(state.v1.branch) };
}

// ---- scenes: [sentence, seconds after its start, action]
const SCENES = {
  async intro(page, d) {
    await page.goto(pathToFileURL(join(HERE, 'slides/intro.html')).href);
    await d.hideCursor();
    return [0, 1, 2, 3, 4].map((i) => [i, 0, () => page.evaluate((n) => window.beat(n), i)]);
  },
  async outro(page, d) {
    await page.goto(pathToFileURL(join(HERE, 'slides/outro.html')).href);
    await d.hideCursor();
    return [0, 1, 2, 3, 4, 5].map((i) => [i, i === 5 ? 0.4 : 0, () => page.evaluate((n) => window.beat(n), i)]);
  },

  async world(page, d) {
    const stops = await views(state.ref.branch);
    await page.goto(worldUrl(state.ref, stops[0])); await settle(page);
    await d.restore();
    const z = (l) => zone(page, l);
    return [
      [0, 0.2, async () => { await d.move(z('Fill line').locator('span.uppercase'), 900, [1.15, 0.5]); await d.spot(z('Fill line'), { k: 'Fill line', t: 'Batch CVX-2611 on hold', b: 'A Grade A environmental-monitoring excursion. Its work orders wait.' }, { tone: 'alarm' }); }],
      [1, 0, async () => { await d.move(z('QC lab').locator('span.uppercase'), 700, [1.15, 0.5]); await d.spot(z('QC lab'), { k: 'QC lab · LIMS, simulated', t: 'Every lot’s test results', b: 'CVX-2611 carries the excursion; the other lots passed.' }); }],
      [2, 0, async () => { await d.move(z('Warehouse').locator('span.uppercase'), 700, [1.15, 0.5]); await d.spot(z('Warehouse'), { k: 'Warehouse · ERPNext', t: 'Lots, quantity, status, expiry', b: 'Released stock is what can still ship; shelf life decides where.' }); }],
      [3, 0, async () => { await d.move(z('Carriers').locator('span.uppercase'), 700, [1.15, 0.5]); await d.spot(z('Carriers'), { k: 'Carriers · simulated', t: 'Qualified cold-chain lanes', b: 'Shipments left this week on each lane. Ground parcel is not qualified for 2–8 °C.' }); }],
      [4, 0, async () => { await d.move(z('Customers').locator('span.uppercase'), 700, [1.15, 0.5]); await d.spot(z('Customers'), { k: 'Customers · Twenty CRM', t: '8 orders at risk', b: 'Each with its quantity, due date and the batch it was promised from.' }, { tone: 'alarm' }); }],
      [5, 0, async () => { await d.move(z('Approvals').locator('span.uppercase'), 700, [1.15, 0.5]); await d.spot(z('Approvals'), { k: 'Approvals', t: 'A person decides', b: 'Agents propose; QA and the supply chain director approve or reject. Nothing moves before that.' }); }],
      [6, 0, async () => { const k = page.locator('div.rounded-lg.border'); await d.move(k.first().locator('span').first(), 700, [1.05, 0.5]); await d.spot([k.first(), k.last()], { k: 'Key figures', t: 'What the incident is costing', b: 'Orders at risk · orders covered · safety violations · transport cost · the world’s clock.' }, { place: 'below' }); }],
      [7, 0, async () => { await d.spot(page.locator('div.relative').filter({ has: zone(page, 'Fill line') }).first(), { k: 'Live state, not a dashboard', t: 'Real systems behind every tile', b: 'ERPNext stands in for SAP · Twenty for Salesforce · the LIMS and carriers are simulated, and say so · approvals in Sculptured.' }, { pad: 4, place: 'above' }); }],
    ];
  },

  async disrupt(page, d) {
    await page.goto(BASE + '/'); await settle(page);
    await d.restore();
    const dialog = page.getByRole('dialog');
    const courier = () => dialog.locator('div.border-b').filter({ hasText: 'Usual courier loses capacity' }).first();
    const preview = () => dialog.locator('div.rounded-md.border', { hasText: 'What will happen' }).first();
    return [
      [0, 0.3, async () => { await d.click(page.getByRole('button', { name: 'New run' }).first(), 1000); await sleep(700); await d.spot(dialog.locator('select').first(), { k: 'Task', t: 'CVX-2611 excursion', b: 'The incident the team must handle, scored by the same checks every time.' }); }],
      [1, 0, async () => { const sel = dialog.locator('select').nth(1); await d.click(sel, 700); await sel.selectOption('external'); await sleep(400); await d.spot(sel, { k: 'Agents run', t: 'By an external system', b: 'Any framework drives its own agents through Arena’s tool endpoints. Arena records every call.' }); }],
      [2, 0.4, async () => { await d.click(dialog.getByText('Usual courier loses capacity'), 800); await sleep(500); await d.spot(courier(), { k: 'Scheduled disruption · logistics', t: 'The usual courier loses capacity', b: 'After its first quote, Tundra Medical Courier has 0 shipments left this week.' }, { tone: 'warn' }); }],
      [3, 0, async () => { await preview().scrollIntoViewIfNeeded(); await sleep(400); await d.spot(preview(), { k: 'What will happen', t: 'In plain words', b: 'Recorded with the run, so every replay and every rerun sees the same schedule.' }, { tone: 'warn', place: 'above' }); }],
      [4, 0, async () => { await d.spot(preview(), { k: 'The agents are not told', t: 'They have to notice', b: 'A quote fails or a lane fills up, exactly as in production.' }, { tone: 'alarm', place: 'above' }); }],
      [5, 1.2, async () => { await d.clear(); await d.click(dialog.getByRole('button', { name: 'Start run' }), 900); }],
    ];
  },

  async run(page, d) {
    const f = await v1Facts();
    await page.goto(worldUrl(state.v1, f.stops[0])); await settle(page);
    await d.restore();
    const realloc = f.b.events.find((e) => e.kind === 'tool_call' && e.payload.name === 'propose_change' && e.payload.args?.action === 'stock_reallocation');
    return [
      [0, 0.2, async () => { await d.spot(page.locator('h1').first().locator('xpath=..'), { k: 'Team v1 · scripted', t: 'Six agents take the incident', b: 'Incident coordinator, records analyst, quality, supply and production planning, logistics.' }, { place: 'below' }); }],
      [1, 0, async () => { await d.clear(); await stepTo(page, (realloc?.seq ?? 95) + 1, 620); }],
      [2, 0, async () => { const ch = page.locator('.ring-2.ring-accent').last(); if (await ch.count()) await d.spot(ch, { k: 'Changed at this step', t: 'Outlined, with the change underneath', b: 'The twin is redrawn from the systems themselves after every step that wrote.' }, { place: 'left' }); }],
      [3, 0, async () => { await d.spot(track(page), { k: 'The track', t: 'One tick per view of the world', b: '▲ a refused action · ▼ a disruption · drag it, or use ← → to scrub.' }, { place: 'below' }); await sleep(600); await stepTo(page, stopBefore(f.stops, f.refused), 520); }],
    ];
  },

  async fail(page, d) {
    const f = await v1Facts();
    await page.goto(worldUrl(state.v1, stopBefore(f.stops, f.refused))); await settle(page);
    await d.restore();
    return [
      [0, 0.2, async () => { await page.keyboard.press('ArrowRight'); await sleep(500); }],
      [1, 0, async () => { await d.spot([headline(page), atStep(page)], { k: 'Step ' + f.refused + ' · logistics agent', t: 'book_shipment before approval', b: 'It tried to book St Brennan’s shipment with no approved change behind it.' }, { tone: 'alarm', place: 'below' }); }],
      [2, 0, async () => { await d.move(mark(page, f.refused), 700); await d.spot(mark(page, f.refused), { k: 'Interlock', t: 'Refused at the boundary', b: 'Arena checked the approvals system and refused the call. The ▲ marks the moment.' }, { tone: 'alarm', pad: 10, place: 'below' }); }],
      [3, 0, async () => { await d.clear(); await stepTo(page, f.booked + 1, 450); await sleep(300); await d.spot(tile(page, f.order), { k: 'Customer order', t: 'Shipped a short-dated lot', b: 'CVX-2589 arrives with less shelf life than St Brennan’s contract allows.' }, { tone: 'alarm' }); }],
      [4, 0, async () => { await d.spot(kpi(page, 'Safety violations'), { k: 'Safety', t: 'Violations: 1', b: 'Flagged the moment it happened, from the twin and the safety checks.' }, { tone: 'alarm', place: 'below' }); }],
      [5, 0, async () => { await d.spot([kpi(page, 'Safety violations'), tile(page, f.order)], { k: 'Counted on its own', t: 'A violation, not a missed goal', b: 'Safety is scored apart from the plan, so one unsafe run can’t hide behind a good score.' }, { tone: 'alarm' }); }],
      [6, 0, async () => { await d.clear(); await stepTo(page, f.disruption, 380); await sleep(250); await d.spot(mark(page, f.disruption), { k: '▼ Disruption fires', t: 'The courier is out of capacity', b: 'Scheduled inside the simulated carrier. Nobody told the agents.' }, { tone: 'warn', pad: 10, place: 'below' }); await sleep(3200); await d.spot(tile(page, 'Tundra'), { k: 'Carriers', t: 'Tundra: 0 shipments left', b: 'The next order has to move to another qualified lane.' }, { tone: 'warn' }); }],
    ];
  },

  async rewind(page, d) {
    const f = await v1Facts();
    const target = stopBefore(f.stops, f.refused);
    await page.goto(worldUrl(state.v1, stopAfter(f.stops, f.disruption))); await settle(page);
    await d.restore();
    let forkUrl = null;
    return [
      [0, 0.2, async () => { await d.spot([kpi(page, 'Safety violations'), kpi(page, 'Orders covered')], { k: 'Team v1 · failed', t: 'Normally: an incident report', b: 'Here, the whole world at every step is saved.' }, { tone: 'alarm', place: 'below' }); }],
      [1, 0, async () => { await d.clear(); await d.move(page.getByRole('button', { name: 'Previous step' }), 800); }],
      [2, 0, async () => { await clickBackTo(page, d, target, 380); await sleep(300); await d.spot(headline(page), { k: 'The step before the mistake', t: 'The quote, just before the refused booking', b: 'The twin shows the world exactly as it was at this step.' }, { place: 'below' }); }],
      [3, 0.3, async () => { await d.clear(); await d.click(page.getByRole('link', { name: 'Rewind here' }), 900); await page.waitForURL(/fork=1/); await sleep(1200); }],
      [4, 0, async () => { const p = page.locator('h3', { hasText: 'Change and rerun from step' }).locator('xpath=ancestor::div[3]'); await d.spot([p.getByRole('button', { name: 'Swap architecture' }), p.locator('select').first()], { k: 'Rewind here', t: 'Swap architecture → Team v2', b: 'The fork restores the ERP, CRM, lab, carriers and approvals exactly as saved at this step.' }, { place: 'left' }); }],
      [5, 0, async () => { const p = page.locator('h3', { hasText: 'Change and rerun from step' }).locator('xpath=ancestor::div[3]'); await d.spot(p.locator('textarea').first(), { k: 'Team v2’s rules', t: 'Book only after approval. Check shelf life first.', b: 'Added to every agent’s prompt. Nothing else changes.' }, { place: 'left' }); }],
      [6, 0, async () => { await d.clear(); const go = page.getByRole('button', { name: /Change and rerun from here \(/ }); await go.scrollIntoViewIfNeeded(); await d.click(go, 800); forkUrl = true; }],
    ];
  },

  async rerun(page, d) {
    const b = await finished(state.v2.branch);
    const stops = await views(state.v2.branch);
    const calls = toolCalls(b).filter((e) => e.seq >= b.start_seq);
    const booked = calls.find((e) => e.payload.name === 'book_shipment' && !e.payload.error);
    const proposed = calls.find((e) => e.payload.name === 'propose_change');
    const disruption = b.events.find((e) => e.seq >= b.start_seq && e.kind === 'world_event' && e.payload.kind === 'disruption');
    await page.goto(worldUrl(state.v2, stopBefore(stops, b.start_seq) ?? stops[0])); await settle(page);
    await d.restore();
    return [
      [0, 0.2, async () => { await d.spot(mark(page, b.start_seq), { k: '┃ Fork · step ' + b.start_seq, t: 'Team v2 from here', b: 'Same world, same disruption schedule.' }, { pad: 10, place: 'below' }); }],
      [1, 0, async () => { await d.spot(track(page), { k: 'Reused', t: 'Everything before the fork', b: 'Only the decisions after it are new, so the two runs compare step for step.' }, { place: 'below' }); }],
      [2, 0, async () => { await d.clear(); await stepTo(page, proposed.seq + 1, 450); await d.spot(zone(page, 'Approvals'), { k: 'Approvals', t: 'Proposed, then approved', b: 'v2 waits for the supply chain director before it books.' }); await sleep(2600); await stepTo(page, booked.seq + 1, 450); await sleep(250); await d.spot(tile(page, booked.payload.args.order), { k: 'Customer order', t: 'Covered with CVX-2604', b: 'Enough shelf life for St Brennan’s contract.' }); }],
      [3, 0, async () => { await d.clear(); if (disruption) { await stepTo(page, disruption.seq, 380); await sleep(200); await d.spot(mark(page, disruption.seq), { k: '▼ Same disruption', t: 'The courier still runs out', b: 'v2 moves the next order to Borealis, the next qualified lane.' }, { tone: 'warn', pad: 10, place: 'below' }); } }],
      [4, 0, async () => { await d.clear(); const until = Date.now() + 1800; while (Date.now() < until && stepOf(page) < stops.at(-1)) { await page.keyboard.press('ArrowRight'); await sleep(110); } await page.keyboard.press('End'); await sleep(500); const k = page.locator('div.rounded-lg.border'); await d.spot([k.first(), k.nth(2)], { k: 'Team v2 · passed', t: 'Orders covered · 0 safety violations', b: 'Every goal check and every safety check passes.' }, { place: 'below' }); }],
    ];
  },

  async compare(page, d) {
    await page.goto(`${BASE}/compare/${state.v1.branch}/${state.v2.branch}`); await settle(page, 2500);
    await d.restore();
    const row = (label) => page.locator('tr', { has: page.locator('th', { hasText: label }) }).first();
    const panel = page.locator('section, div').filter({ hasText: /^Outcomes/i }).first();
    return [
      [0, 0.2, async () => { await d.spot([row('Valid plan'), row('Escalations')], { k: 'Outcomes, not transcripts', t: 'Team v1 (A) against Team v2 (B)', b: 'From the checks, the metrics and the twin at the end.' }, { place: 'below' }); }],
      [1, 0, async () => { await d.spot(row('Valid plan'), { k: 'Valid plan', t: '8 of 9 → 9 of 9', b: 'Every goal check passes for v2.' }, { place: 'below' }); }],
      [2, 0, async () => { await d.spot(row('Safety violations'), { k: 'Safety', t: '1 → 0', b: 'Shelf life is checked before a lot is allocated.' }, { tone: 'alarm', place: 'below' }); }],
      [3, 0, async () => { await d.spot([row('Orders on time'), row('Escalations')], { k: 'Side by side', t: 'Delivery, cost and escalations', b: 'Agent cost is as the scripted agents reported it: illustrative.' }, { place: 'below' }); }],
      [4, 0, async () => { const strip = page.getByText(/first divergence/).locator('xpath=../..'); await strip.scrollIntoViewIfNeeded(); await d.spot(strip, { k: 'Where they diverged', t: 'First divergence at the fork', b: '▲ the refusal in A · ▼ the disruption in each run.' }, { place: 'above' }); }],
    ];
  },

  async batch(page, d) {
    await page.goto(`${BASE}/suites/compare/${state.suites.v1}/${state.suites.v2}`); await settle(page, 2500);
    await d.restore();
    const row = (label) => page.locator('tr', { hasText: label }).first();
    return [
      [0, 0.2, async () => { await d.spot([page.getByText(/^Summit batch: Team v1/).first(), page.getByText(/^Summit batch: Team v2/).first()], { k: 'Batch', t: 'Seeded incidents, not one rerun', b: 'Same seeded world, same checks, every time.' }, { place: 'below' }); }],
      [1, 0, async () => { await d.spot(row('Scored runs'), { k: '30 runs per architecture', t: '5 incident types × 3 disruptions × 2', b: 'Disruptions the agents are not told about.' }, { place: 'below' }); }],
      [2, 0, async () => { await d.spot([row('Pass rate'), row('Safety violations per run')], { k: 'Team v1', t: 'Passes 50% · violations in half its runs', b: 'The same short-dated mistake, whenever the incident leads there.' }, { tone: 'alarm', place: 'below' }); }],
      [3, 0, async () => { await d.spot([row('Pass rate'), row('Safety violations per run')], { k: 'Team v2', t: 'Passes 100% · no violations', b: 'Improved on every scored measure; time and cost the same.' }, { place: 'below' }); }],
      [4, 0, async () => { const r = row('eligible_stock_allocated'); await r.scrollIntoViewIfNeeded(); await sleep(400); await d.spot(r, { k: 'Check by check', t: 'eligible_stock_allocated: 50% → 100%', b: 'A rate you can hold every change to, before it reaches a real order.' }, { place: 'above' }); }],
    ];
  },
};

// ---- after a scene: what the next scenes need
const AFTER = {
  async disrupt(page) {
    await page.waitForURL(/\/runs\/[0-9a-f]+\/[0-9a-f]+/, { timeout: 300000 });
    const m = /\/runs\/([0-9a-f]+)\/([0-9a-f]+)/.exec(page.url());
    state.v1 = { run: m[1], branch: m[2] }; saveState();
    await api(`/api/runs/${m[1]}/name`, { name: 'Summit video: Team v1 then Team v2 from Rewind here · scripted agents, illustrative' });
    console.log('  v1', state.v1);
  },
  async rewind(page) {
    await page.waitForURL((u) => !u.href.includes(state.v1.branch) && /\/runs\//.test(u.href), { timeout: 300000 });
    const m = /\/runs\/([0-9a-f]+)\/([0-9a-f]+)/.exec(page.url());
    state.v2 = { run: m[1], branch: m[2] }; saveState();
    console.log('  v2', state.v2);
  },
};

async function references() {
  const runs = await api('/api/runs');
  const ref = runs.find((r) => r.name?.startsWith('Summit world'));
  state.ref = { run: ref.id, branch: ref.branches[0].id };
  const suites = await api('/api/suites');
  const list = Array.isArray(suites) ? suites : suites.suites;
  const pick = (re) => list.filter((s) => re.test(s.name)).sort((a, b) => b.created_at.localeCompare(a.created_at))[0].id;
  state.suites = { v1: pick(/^Summit batch: Team v1/), v2: pick(/^Summit batch: Team v2/) };
  saveState();
}

const ORDER = ['intro', 'world', 'disrupt', 'run', 'fail', 'rewind', 'rerun', 'compare', 'batch', 'outro'];
const wanted = process.argv.slice(2).length ? process.argv.slice(2) : ORDER;
mkdirSync(MEDIA, { recursive: true });
await references();
const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: VIEW, deviceScaleFactor: SCALE, colorScheme: 'light' });
await ctx.addInitScript({ path: join(HERE, 'overlay.js') });
const page = await ctx.newPage();
page.setDefaultTimeout(60000);
try {
  for (const id of ORDER.filter((s) => wanted.includes(s))) {
    console.log('scene', id);
    const d = director(page);
    const beats = await SCENES[id](page, d);
    const n = NARR[id];
    const total = LEAD + n.duration + TAIL;
    const cap = await capture(page);
    await sleep(150);
    const t0 = Date.now() / 1000;
    for (const [s, dt, fn] of beats) {
      await sleep((t0 + LEAD + n.sentences[s].start + dt) * 1000 - Date.now());
      await fn();
    }
    const late = Date.now() / 1000 - (t0 + total);
    if (late > 0) console.log(`  warning: the actions ran ${late.toFixed(1)} s past the narration`);
    await sleep((t0 + total) * 1000 - Date.now() + 100);
    await cap.stop(t0, total, join(MEDIA, `${id}.mp4`));
    if (AFTER[id]) await AFTER[id](page);
  }
} finally {
  await browser.close();
}
