// Screenshots (and short screen recordings) of web UIs for the showcase video: Arena's UI on the
// recorded runs, and Twenty or ERPNext on a world's data. Read only: it only opens pages and scrolls.
//
//   node capture.mjs shots.json OUT_DIR
//
// shots.json: {"base": "http://127.0.0.1:4510", "width": 1280, "height": 720, "scale": 1.5, "shots": [
//   {"name": "runs", "path": "/", "wait": "text=Runs", "scroll": 0, "record": false,
//    "click": "text=Compare", "settle_ms": 1500}]}
// Optional per shot: "timeout_ms" (default 60000), "warm" (an API path fetched first, see warm()),
// "hover", "scheme" ("dark" or "light"). A shot writes OUT_DIR/<name>.png; with "record": true it also writes OUT_DIR/<name>.webm, a
// recording of the page scrolling down by "scroll" pixels over "record_s" seconds.
// Playwright: `npm i playwright` here, or NODE_PATH pointing at a node_modules that has it.
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { chromium } = require('playwright');

async function settle(page, shot) {
  if (shot.wait) await page.waitForSelector(shot.wait);
  if (shot.click) await page.click(shot.click, { timeout: 15000 });
  await page.waitForTimeout(shot.settle_ms ?? 1500);
}

async function scrollSlowly(page, pixels, seconds) {
  const steps = Math.max(1, Math.round(seconds * 30));
  for (let i = 0; i < steps; i++) {
    await page.mouse.wheel(0, pixels / steps);
    await page.waitForTimeout(1000 / 30);
  }
}

// A slow page's API answer is asked for once first (`warm`: an API path), so a recording shows
// the page, not its loading: Arena's integrations keep what they read from saves.
async function warm(cfg, shot) {
  if (!shot.warm) return;
  const r = await fetch((shot.base ?? cfg.base) + shot.warm, { signal: AbortSignal.timeout(shot.timeout_ms ?? 60000) });
  if (!r.ok) throw new Error(`warming ${shot.warm}: ${r.status}`);
}

async function capture(browser, cfg, shot, out) {
  await warm(cfg, shot);
  const ctx = await openContext(browser, cfg, shot, out);
  try {
    await shoot(ctx, cfg, shot, out);
  } finally {
    const video = ctx.pages()[0]?.video();
    await ctx.close();
    if (video) await keepVideo(video, shot, out);
  }
  console.log(`ok ${shot.name}`);
}

async function openContext(browser, cfg, shot, out) {
  // "scale" enlarges the page: a 1280x720 viewport at 1.5 gives 1920x1080 images with text big
  // enough to read in a video.
  const size = { width: cfg.width ?? 1920, height: cfg.height ?? 1080 };
  const scale = cfg.scale ?? 1;
  const ctx = await browser.newContext({
    viewport: size, deviceScaleFactor: scale, colorScheme: shot.scheme ?? cfg.scheme ?? 'dark',
    // Playwright records at the viewport's size whatever the scale (a larger frame leaves the page
    // in a corner), so recordings are viewport-sized and the editor scales them up.
    recordVideo: shot.record ? { dir: out, size } : undefined,
  });
  return ctx;
}

// The recording, as OUT_DIR/<name>.webm; none is kept for a shot that failed.
async function keepVideo(video, shot, out) {
  const path = await video.path();
  if (existsSync(join(out, `${shot.name}.png`))) renameSync(path, join(out, `${shot.name}.webm`));
  else rmSync(path, { force: true });
}

async function shoot(ctx, cfg, shot, out) {
  rmSync(join(out, `${shot.name}.png`), { force: true });
  const page = await ctx.newPage();
  // A slow page (Arena's compare reads both worlds' final saves) gets its own `timeout_ms`.
  page.setDefaultTimeout(shot.timeout_ms ?? 60000);
  await page.goto((shot.base ?? cfg.base) + shot.path, { waitUntil: 'networkidle', timeout: shot.timeout_ms ?? 60000 });
  await settle(page, shot);
  if (shot.hover) await page.hover(shot.hover);
  await page.screenshot({ path: join(out, `${shot.name}.png`) });
  if (shot.record) await scrollSlowly(page, shot.scroll ?? 800, shot.record_s ?? 6);
}

const [shotsFile, out] = process.argv.slice(2);
if (!shotsFile || !out) { console.error('usage: node capture.mjs shots.json OUT_DIR'); process.exit(2); }
const cfg = JSON.parse(readFileSync(shotsFile, 'utf8'));
mkdirSync(out, { recursive: true });
const browser = await chromium.launch();
let failed = 0;
for (const shot of cfg.shots) {
  try { await capture(browser, cfg, shot, out); } catch (e) { failed++; console.error(`failed ${shot.name}: ${e.message}`); }
}
await browser.close();
process.exit(failed ? 1 : 0);
