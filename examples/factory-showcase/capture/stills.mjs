// node stills.mjs shots.json OUT: stills (1280x720 at 1.5x), banner hidden; steps: click, wheel, wait, press, hover.
import { createRequire } from 'node:module'; import { readFileSync, mkdirSync } from 'node:fs';
const { chromium } = createRequire(import.meta.url)('playwright');
const [file, out] = process.argv.slice(2); const cfg = JSON.parse(readFileSync(file, 'utf8')); mkdirSync(out, { recursive: true });
const b = await chromium.launch();
// The union box of `focus` selectors (page coords), padded and grown to 16:9; capped at 16:9 from its top.
async function focusClip(p, s) {
  if (!s.focus) return undefined;
  const boxes = [];
  for (const sel of [].concat(s.focus)) {
    const l = p.locator(sel).nth(s.nth ?? 0); await l.scrollIntoViewIfNeeded(); await p.waitForTimeout(300);
    const r = await l.evaluate((e) => { const b = e.getBoundingClientRect(); return [b.left + scrollX, b.top + scrollY, b.right + scrollX, b.bottom + scrollY]; });
    boxes.push(r);
  }
  const pad = s.pad ?? 24;
  let [x0, y0, x1, y1] = [Math.min(...boxes.map(b => b[0])) - pad, Math.min(...boxes.map(b => b[1])) - pad,
                          Math.max(...boxes.map(b => b[2])) + pad, Math.max(...boxes.map(b => b[3])) + pad];
  let w = x1 - x0, h = y1 - y0;
  if (s.noAspect) return { x: Math.max(0, x0), y: Math.max(0, y0), width: w, height: h };
  if (w / h > 16 / 9) { const nh = w * 9 / 16; y0 -= (nh - h) / 2; h = nh; } else { const nw = h * 16 / 9; if (nw > 1280) { w = 1280; h = 720; x0 = 0; } else { x0 -= (nw - w) / 2; w = nw; } }
  x0 = Math.max(0, Math.min(x0, 1280 - w)); y0 = Math.max(0, y0);
  return { x: x0, y: y0, width: w, height: h };
}
for (const s of cfg.shots) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 720 }, deviceScaleFactor: 2, colorScheme: 'dark' });
  const p = await ctx.newPage(); p.setDefaultTimeout(s.timeout_ms ?? 90000);
  try {
    await p.goto((s.base ?? cfg.base) + s.path, { waitUntil: 'networkidle', timeout: s.timeout_ms ?? 90000 });
    await p.addStyleTag({ content: '.status-line{display:none!important}' + (cfg.css ?? '') });
    await p.waitForTimeout(s.settle_ms ?? 2000);
    for (const st of s.steps ?? []) {
      if (st.click) await p.click(st.click);
      if (st.hover) await p.hover(st.hover);
      if (st.press) await p.keyboard.press(st.press);
      if (st.wheel) { await p.mouse.move(640, 400); await p.mouse.wheel(0, st.wheel); }
      if (st.scrollTo) await p.locator(st.scrollTo).first().scrollIntoViewIfNeeded();
      await p.waitForTimeout(st.wait ?? 1200);
    }
    const clip = s.clip ? { x: s.clip[0], y: s.clip[1], width: s.clip[2], height: s.clip[3] } : await focusClip(p, s);
    await p.screenshot({ path: `${out}/${s.name}.png`, fullPage: !!clip, clip }); console.log('ok', s.name);
  } catch (e) { console.log('FAILED', s.name, e.message.split('\n')[0]); }
  await ctx.close();
}
await b.close();
