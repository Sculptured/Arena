// Where the twin's overview draws things: areas as zones left to right in process order
// (fill line, QC lab, warehouse, carriers, customers, then any other kind), approvals as a rail on
// the right, items as tiles in their zone, and flows as arrows from tile to tile.
import type { AreaKind, TwinArea, TwinFlow, TwinView } from './types';

export const PROCESS_ORDER: AreaKind[] = ['plant', 'lab', 'warehouse', 'carriers', 'customers'];
export const RAIL_KINDS: AreaKind[] = ['approvals'];

export const ZONE_HEAD = 30;
export const ZONE_PAD = 8;
export const TILE_H = 68;
/** A changed tile carries one more line: what changed. */
export const TILE_H_CHANGED = 86;
/** Room for each flow on the bus that runs under the zones between its ends. */
export const BUS_LANE = 15;
/** Room between the tiles above and the bus's first lane, for its label. */
export const BUS_GAP = 8;
export const TILE_GAP = 6;
/** Room between zones, where arrows run. */
export const ZONE_GAP = 20;
/** Below this tile width the overview stacks its zones instead. */
export const MIN_TILE_W = 148;
/** A zone with more items than this splits into two columns. */
export const SPLIT_AT = 6;
const RAIL_WEIGHT = 1.15;

export interface Rect { x: number; y: number; w: number; h: number }
export interface ZoneBox extends Rect { area: TwinArea; cols: number; rail: boolean }
export interface TileBox extends Rect { id: string; zone: string }
/** A flow drawn: its path, its ends, and where its label sits when it runs along the bus. */
export interface Arrow { flow: TwinFlow; d: string; x1: number; y1: number; x2: number; y2: number; label?: { x: number; y: number } }
export interface TwinLayout { zones: ZoneBox[]; tiles: TileBox[]; arrows: Arrow[]; width: number; height: number }

const rank = (kind: AreaKind) => {
  const i = PROCESS_ORDER.indexOf(kind);
  return i < 0 ? PROCESS_ORDER.length : i;
};

/** The zones in process order, and the rail's (approvals). Unknown kinds follow the known ones, as given. */
export function splitAreas(view: TwinView): { main: TwinArea[]; rail: TwinArea[] } {
  const main = view.areas.filter((a) => !RAIL_KINDS.includes(a.kind));
  const sorted = main.map((a, i) => ({ a, i })).sort((x, y) => rank(x.a.kind) - rank(y.a.kind) || x.i - y.i).map((x) => x.a);
  return { main: sorted, rail: view.areas.filter((a) => RAIL_KINDS.includes(a.kind)) };
}

interface Column { area: TwinArea; cols: number; weight: number; rail: boolean }

function columnsOf(view: TwinView): Column[] {
  const { main, rail } = splitAreas(view);
  const cols = (a: TwinArea) => (a.items.length > SPLIT_AT ? 2 : 1);
  return [
    ...main.map((area) => ({ area, cols: cols(area), weight: cols(area), rail: false })),
    ...rail.map((area) => ({ area, cols: 1, weight: RAIL_WEIGHT, rail: true })),
  ];
}

/** The width of one unit of zone (a one-column zone), or 0 when the view has no areas. */
function unitWidth(columns: Column[], width: number): number {
  const weight = columns.reduce((s, c) => s + c.weight, 0);
  if (!weight) return 0;
  return (width - ZONE_GAP * (columns.length - 1)) / weight;
}

/** True when the overview fits `width` with tiles wide enough to read; else it stacks. */
export function fitsSideBySide(view: TwinView, width: number): boolean {
  const columns = columnsOf(view);
  const unit = unitWidth(columns, width);
  return columns.length > 0 && unit - ZONE_PAD * 2 >= MIN_TILE_W;
}

/** One zone's tiles, row by row: a row is as tall as its tallest tile. */
function placeTiles(zone: ZoneBox, changed: Set<string>): { tiles: TileBox[]; bottom: number } {
  const tileW = (zone.w - ZONE_PAD * 2 - TILE_GAP * (zone.cols - 1)) / zone.cols;
  const tiles: TileBox[] = [];
  let y = zone.y + ZONE_HEAD + ZONE_PAD;
  for (let i = 0; i < zone.area.items.length; i += zone.cols) {
    const row = zone.area.items.slice(i, i + zone.cols);
    const h = Math.max(...row.map((it) => (changed.has(it.id) ? TILE_H_CHANGED : TILE_H)));
    row.forEach((it, c) => tiles.push({ id: it.id, zone: zone.area.id, x: zone.x + ZONE_PAD + c * (tileW + TILE_GAP), y, w: tileW, h }));
    y += h + TILE_GAP;
  }
  return { tiles, bottom: y - TILE_GAP + ZONE_PAD };
}

/** An arrow from the right edge of one tile to the left edge of the other (or back, when it runs leftwards). */
export function arrowBetween(flow: TwinFlow, from: Rect, to: Rect): Arrow {
  const forward = to.x >= from.x + from.w;
  const x1 = forward ? from.x + from.w : from.x;
  const x2 = forward ? to.x : to.x + to.w;
  const y1 = from.y + from.h / 2;
  const y2 = to.y + to.h / 2;
  const bend = Math.max(40, Math.abs(x2 - x1) / 2) * (forward ? 1 : -1);
  return { flow, x1, y1, x2, y2, d: `M${x1},${y1} C${x1 + bend},${y1} ${x2 - bend},${y2} ${x2},${y2}` };
}

/** An orthogonal path through `pts` with its corners rounded by up to `r`. */
export function roundedPath(pts: [number, number][], r = 5): string {
  let d = `M${pts[0][0]},${pts[0][1]}`;
  for (let i = 1; i < pts.length - 1; i++) {
    const [px, py] = pts[i - 1];
    const [x, y] = pts[i];
    const [nx, ny] = pts[i + 1];
    const k = Math.min(r, Math.hypot(x - px, y - py) / 2, Math.hypot(nx - x, ny - y) / 2);
    const ax = x - Math.sign(x - px) * k, ay = y - Math.sign(y - py) * k;
    const bx = x + Math.sign(nx - x) * k, by = y + Math.sign(ny - y) * k;
    d += ` L${ax},${ay} Q${x},${y} ${bx},${by}`;
  }
  const [lx, ly] = pts[pts.length - 1];
  return `${d} L${lx},${ly}`;
}

interface Route { flow: TwinFlow; from: TileBox; to: TileBox; zones: ZoneBox[] }

/**
 * Flows whose ends have zones between them (a lot to an order, past the carriers) run as pipes:
 * out into the gutter, down to a bus under the zones between, across, and up into the target.
 * Each has its own lane, so none overlap, and its label sits on the bus.
 */
function busArrows(routes: Route[], busTop: number): Arrow[] {
  const spread = Math.min(4, (ZONE_GAP - 6) / Math.max(1, routes.length));
  return routes.map((r, i) => {
    const off = (i - (routes.length - 1) / 2) * spread;
    const first = r.zones[0], last = r.zones[r.zones.length - 1];
    const g1 = first.x - ZONE_GAP / 2 + off, g2 = last.x + last.w + ZONE_GAP / 2 + off;
    const y = busTop + BUS_GAP + i * BUS_LANE + BUS_LANE / 2;
    const x1 = r.from.x + r.from.w, y1 = r.from.y + r.from.h / 2, x2 = r.to.x, y2 = r.to.y + r.to.h / 2;
    const d = roundedPath([[x1, y1], [g1, y1], [g1, y], [g2, y], [g2, y2], [x2, y2]]);
    return { flow: r.flow, d, x1, y1, x2, y2, label: { x: first.x + ZONE_PAD, y: y - 3 } };
  });
}

/** The zones strictly between two tiles' zones, left to right. */
function between(zones: ZoneBox[], a: TileBox, b: TileBox): ZoneBox[] {
  const ia = zones.findIndex((z) => z.area.id === a.zone), ib = zones.findIndex((z) => z.area.id === b.zone);
  return ia >= 0 && ib > ia + 1 ? zones.slice(ia + 1, ib) : [];
}

/** Each flow whose ends are drawn: routed along the bus, or a curve between neighbours. */
function flowRoutes(view: TwinView, zones: ZoneBox[], tiles: TileBox[]): { direct: Arrow[]; routes: Route[] } {
  const at = new Map(tiles.map((t) => [t.id, t]));
  const direct: Arrow[] = [];
  const routes: Route[] = [];
  for (const f of view.flows) {
    const a = at.get(f.from), b = at.get(f.to);
    if (!a || !b) continue;
    const mid = between(zones, a, b);
    if (mid.length) routes.push({ flow: f, from: a, to: b, zones: mid });
    else direct.push(arrowBetween(f, a, b));
  }
  routes.sort((x, y) => x.from.y - y.from.y || x.to.y - y.to.y);
  return { direct, routes };
}

/** The whole overview at `width`: zones of one height, their tiles, and an arrow per flow whose ends are drawn. */
export function twinLayout(view: TwinView, width: number, changed: Set<string> = new Set()): TwinLayout {
  const columns = columnsOf(view);
  const unit = unitWidth(columns, width);
  let x = 0;
  const zones: ZoneBox[] = columns.map((c) => {
    const z: ZoneBox = { area: c.area, cols: c.cols, rail: c.rail, x, y: 0, w: unit * c.weight, h: 0 };
    x += z.w + ZONE_GAP;
    return z;
  });
  const placed = zones.map((z) => placeTiles(z, changed));
  const tiles = placed.flatMap((p) => p.tiles);
  const { direct, routes } = flowRoutes(view, zones, tiles);
  const crossed = new Set(routes.flatMap((r) => r.zones.map((z) => z.area.id)));
  const busTop = Math.max(0, ...placed.filter((_, i) => crossed.has(zones[i].area.id)).map((p) => p.bottom));
  const height = Math.max(ZONE_HEAD + ZONE_PAD * 2 + TILE_H, ...placed.map((p) => p.bottom), routes.length ? busTop + BUS_GAP + routes.length * BUS_LANE + ZONE_PAD : 0);
  zones.forEach((z) => { z.h = height; });
  return { zones, tiles, arrows: [...direct, ...busArrows(routes, busTop)], width, height };
}

/** The tallest the overview gets over all the views, so scrubbing never makes it jump; a changed row's extra line included. */
export function stableHeight(views: TwinView[], width: number): number {
  return Math.max(0, ...views.map((v) => twinLayout(v, width).height + (TILE_H_CHANGED - TILE_H)));
}
