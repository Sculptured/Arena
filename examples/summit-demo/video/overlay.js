// In-page overlay for the summit video: a cursor, click ripples, and a spotlight that dims the page
// around one element with an explanation card beside it. Injected by record.mjs before every page
// loads; drawn above the app, never intercepting pointer events, so real clicks reach the UI.
(() => {
  if (window.__ov) return;
  const EASE = 'cubic-bezier(.2,.8,.2,1)';
  const css = `
#ov-root{position:fixed;inset:0;pointer-events:none;z-index:2147483646;font-family:Inter,system-ui,sans-serif}
#ov-hole{position:fixed;border-radius:10px;box-shadow:0 0 0 200vmax rgba(8,12,20,.5);border:2.5px solid var(--ov-accent);opacity:0;
  transition:left .7s ${EASE},top .7s ${EASE},width .7s ${EASE},height .7s ${EASE},opacity .45s ease}
#ov-card{position:fixed;width:max-content;max-width:380px;background:#fff;color:#0f172a;border-radius:10px;box-shadow:0 14px 44px rgba(0,0,0,.30);
  padding:13px 16px 14px 16px;border-top:4px solid var(--ov-accent);opacity:0;transform:translateY(8px);
  transition:opacity .35s ease,transform .35s ease,left .7s ${EASE},top .7s ${EASE}}
#ov-card .k{font-size:11px;font-weight:650;letter-spacing:.07em;text-transform:uppercase;color:var(--ov-accent);margin-bottom:5px}
#ov-card .t{font-size:17px;font-weight:650;line-height:1.25}
#ov-card .b{font-size:14px;line-height:1.42;color:#334155;margin-top:5px}
#ov-card.alarm{border-top-color:#c62828} #ov-card.alarm .k{color:#c62828}
#ov-card.warn{border-top-color:#b26a00} #ov-card.warn .k{color:#b26a00}
#ov-hole.alarm{border-color:#c62828} #ov-hole.warn{border-color:#b26a00}
#ov-cursor{position:fixed;left:0;top:0;width:28px;height:28px;transition:transform .8s cubic-bezier(.45,.05,.25,1);will-change:transform;filter:drop-shadow(0 2px 3px rgba(0,0,0,.35))}
#ov-tick{position:fixed;right:0;bottom:0;width:2px;height:2px;animation:ovtick 1s linear infinite}
.ov-ripple{position:fixed;width:36px;height:36px;margin:-18px 0 0 -18px;border-radius:50%;border:3px solid var(--ov-accent);animation:ovr .6s ease-out forwards}
@keyframes ovr{from{transform:scale(.3);opacity:1}to{transform:scale(1.7);opacity:0}}
@keyframes ovtick{from{opacity:.01}to{opacity:.02}}
.status-line{display:none!important}`;
  function mount() {
    if (document.getElementById('ov-root')) return;
    const style = document.createElement('style'); style.textContent = css; document.head.appendChild(style);
    const root = document.createElement('div'); root.id = 'ov-root';
    root.innerHTML = `<div id="ov-hole"></div><div id="ov-card"></div><div id="ov-tick"></div>
<svg id="ov-cursor" viewBox="0 0 24 24"><path d="M4 2.5 L4 19.5 L8.6 15.4 L11.6 22 L14.6 20.7 L11.7 14.3 L18 14.3 Z" fill="#fff" stroke="#111" stroke-width="1.4" stroke-linejoin="round"/></svg>`;
    document.body.appendChild(root);
    const accent = getComputedStyle(document.documentElement).getPropertyValue('--accent').trim();
    root.style.setProperty('--ov-accent', accent || '#2563eb');
    const c = window.__ovCursor || { x: innerWidth * 0.62, y: innerHeight * 0.55 };
    document.getElementById('ov-cursor').style.transform = `translate(${c.x - 4}px,${c.y - 2}px)`;
  }
  const $ = (id) => document.getElementById(id);
  // Where the card goes: below the hole, else above, else beside it; never in the caption band.
  function place(r, card, pref) {
    const W = innerWidth, H = innerHeight, gap = 16, low = H - 150;
    const cw = card.offsetWidth, ch = card.offsetHeight;
    const x = Math.min(Math.max(r.x, 16), W - cw - 16);
    const opts = {
      below: [x, r.y + r.h + gap, r.y + r.h + gap + ch <= low],
      above: [x, r.y - gap - ch, r.y - gap - ch >= 56],
      right: [r.x + r.w + gap, Math.min(Math.max(r.y, 56), low - ch), r.x + r.w + gap + cw <= W - 16],
      left: [r.x - gap - cw, Math.min(Math.max(r.y, 56), low - ch), r.x - gap - cw >= 16],
    };
    const order = [pref, 'below', 'above', 'right', 'left'].filter(Boolean);
    for (const k of order) if (opts[k][2]) return opts[k];
    return [W - cw - 24, 64];
  }
  window.__ov = {
    mount,
    spot(r, card, tone, pref) {
      mount();
      const hole = $('ov-hole'), el = $('ov-card');
      hole.className = tone || ''; el.className = tone || '';
      Object.assign(hole.style, { left: `${r.x}px`, top: `${r.y}px`, width: `${r.w}px`, height: `${r.h}px`, opacity: '1' });
      if (!card) { el.style.opacity = '0'; return; }
      el.innerHTML = (card.k ? `<div class="k">${card.k}</div>` : '') + `<div class="t">${card.t}</div>` + (card.b ? `<div class="b">${card.b}</div>` : '');
      const [x, y] = place(r, el, pref);
      Object.assign(el.style, { left: `${x}px`, top: `${y}px`, opacity: '1', transform: 'translateY(0)' });
    },
    clear() { mount(); $('ov-hole').style.opacity = '0'; Object.assign($('ov-card').style, { opacity: '0', transform: 'translateY(8px)' }); },
    cursor(x, y, ms) {
      mount(); window.__ovCursor = { x, y };
      const cur = $('ov-cursor'); cur.style.transition = `transform ${ms}ms cubic-bezier(.45,.05,.25,1)`;
      cur.style.transform = `translate(${x - 4}px,${y - 2}px)`;
    },
    ripple(x, y) {
      mount(); const d = document.createElement('div'); d.className = 'ov-ripple';
      Object.assign(d.style, { left: `${x}px`, top: `${y}px` }); $('ov-root').appendChild(d); setTimeout(() => d.remove(), 700);
    },
  };
  if (document.body) mount(); else document.addEventListener('DOMContentLoaded', mount);
})();
