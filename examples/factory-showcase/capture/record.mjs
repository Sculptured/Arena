// node record.mjs URL OUT.webm SECONDS [scrollPx]: records a page (banner hidden) for SECONDS.
import { createRequire } from 'node:module'; import { renameSync } from 'node:fs';
const { chromium } = createRequire(import.meta.url)('playwright');
const [url, out, secs, scroll] = process.argv.slice(2);
const b = await chromium.launch(); const size = { width: 1280, height: 720 };
const ctx = await b.newContext({ viewport: size, colorScheme: 'dark', recordVideo: { dir: out + '.d', size } });
const p = await ctx.newPage(); await p.goto(url, { waitUntil: 'networkidle' });
await p.addStyleTag({ content: '.status-line{display:none!important}' });
if (scroll) await p.mouse.wheel(0, Number(scroll));
await p.waitForTimeout(Number(secs) * 1000);
const v = p.video(); await ctx.close(); renameSync(await v.path(), out); await b.close();
