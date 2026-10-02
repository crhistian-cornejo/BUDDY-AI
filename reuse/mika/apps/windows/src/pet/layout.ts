import type { IslandViewName } from "../core/layout";

export type PetMode = "pet" | "radial" | "chat" | "say" | "notice" | "media";

/** Hook decisions and acknowledgements belong beside Mika; conversations use the chat panel. */
export function petModeForView(view: IslandViewName): "notice" | "chat" {
  return view === "approval" || view === "finished" || view === "error" || view === "question" ? "notice" : "chat";
}
export type ArcDirection = "up" | "down" | "left" | "right";
export interface PetFrame {
  width: number;
  height: number;
  cx: number;
  cy: number;
  mode: PetMode;
  arc?: ArcDirection | null;
}

/** Mika's square and where her drawn body sits in it (measured on the canvas: 32..80 of 96). */
export const PET_SIZE = 96;
export const BODY_TOP = 32;
export const BODY_BOTTOM = 80;

/**
 * One ring item with its label (icon circle 34 px + label pill) around its anchor point (the `left`/`top` the CSS gives
 * the button): the box is 64 x 52 px and starts 32 px left of and 21 px above the anchor. The label is capped at
 * 64 px (ellipsis), so this box is what must stay inside the window, labelled or not.
 */
export const RING_ITEM = { left: -32, top: -21, width: 64, height: 52 };
/** Compact items (corners): just a 30 px circle centred on the anchor; the label only appears under the hovered one. */
export const RING_ICON = 30;
/** The native "radial" window is a RING_WINDOW square (Rust `RING` must agree), or the work area if that is smaller. */
export const RING_WINDOW = 340;

export interface Pt { x: number; y: number }
interface Box { left: number; top: number; right: number; bottom: number }

export const ringItemBox = (p: Pt): Box => ({
  left: p.x + RING_ITEM.left, top: p.y + RING_ITEM.top,
  right: p.x + RING_ITEM.left + RING_ITEM.width, bottom: p.y + RING_ITEM.top + RING_ITEM.height,
});
export const ringIconBox = (p: Pt): Box => ({ left: p.x - RING_ICON / 2, top: p.y - RING_ICON / 2, right: p.x + RING_ICON / 2, bottom: p.y + RING_ICON / 2 });
/** Mika's body: her whole 96 px width, only the rows that are drawn (BODY_TOP..BODY_BOTTOM). */
export const petBodyBox = (f: { cx: number; cy: number }): Box => ({
  left: f.cx - PET_SIZE / 2, right: f.cx + PET_SIZE / 2,
  top: f.cy - PET_SIZE / 2 + BODY_TOP, bottom: f.cy - PET_SIZE / 2 + BODY_BOTTOM,
});
const overlaps = (a: Box, b: Box, pad = 0) =>
  a.left < b.right + pad && b.left < a.right + pad && a.top < b.bottom + pad && b.top < a.bottom + pad;

/** Violations of one layout: items outside the window, touching each other, or touching Mika. `compact` = bare circles. */
export function ringProblems(f: PetFrame, points: Pt[], compact = false): number {
  const edge = 4, body = petBodyBox(f);
  let bad = 0;
  points.forEach((p, i) => {
    const b = ringItemBox(p);
    if (b.left < edge || b.top < edge || b.right > f.width - edge || b.bottom > f.height - edge) bad++;
    if (overlaps(compact ? ringIconBox(p) : b, body, 4)) bad++;
    for (let j = i + 1; j < points.length; j++) {
      const q = points[j];
      if (compact ? Math.hypot(p.x - q.x, p.y - q.y) < RING_ICON + 4 : overlaps(b, ringItemBox(q), 2)) bad++;
    }
  });
  return bad;
}

/** Where the ring faces (angle of the arc's middle, y down) and how wide it is. */
export function ringDirection(f: PetFrame): { facing: number; span: number } {
  const NEAR = 168; // the 340 px window is centred on Mika unless the screen edge pushes it: cx < 170 means an edge is near
  const left = f.cx, right = f.width - f.cx, top = f.cy, bottom = f.height - f.cy;
  const h = Math.min(left, right) < NEAR ? (left <= right ? 1 : -1) : 0; // +1: edge on the left, so the ring opens to the right
  const v = Math.min(top, bottom) < NEAR ? (top <= bottom ? 1 : -1) : 0;
  const deg = (a: number) => (a * Math.PI) / 180;
  // Corner: a quarter circle facing diagonally into the screen. Edge: a half circle facing away from it.
  if (h && v) return { facing: Math.atan2(v, h), span: deg(90) };
  if (h) return { facing: h > 0 ? 0 : deg(180), span: deg(180) };
  if (v) return { facing: v > 0 ? deg(90) : deg(-90), span: deg(180) };
  const arc = f.arc ?? "up";
  return { facing: arc === "up" ? deg(-90) : arc === "down" ? deg(90) : arc === "left" ? deg(180) : 0, span: deg(180) };
}

/** `count` points evenly spaced over an arc (ends included); `rings` > 1 alternates radii (outer first). */
function arcPoints(f: PetFrame, count: number, facing: number, span: number, outer: number, rings: number, gap: number): Pt[] {
  return Array.from({ length: count }, (_, i) => {
    const angle = count === 1 ? facing : facing - span / 2 + (i * span) / (count - 1);
    const radius = outer - (i % rings) * gap;
    return { x: f.cx + Math.cos(angle) * radius, y: f.cy + Math.sin(angle) * radius };
  });
}

export interface RingSpots { agents: Pt[]; music: Pt | null; settings: Pt; compact: boolean; radius: number; rings: number }

/**
 * The ring of other agents plus Música (if playing) and Settings, all ordinary items of one arc: Música and Settings
 * take the two ends, the agents the middle, equal angle steps. Along an edge (half circle) every item shows its label;
 * in a corner (quarter circle) there is no room for 64 px labels, so the items are bare 30 px circles with the label
 * only under the hovered one. Always a single radius (about 100 to 135 px from Mika) when it fits; otherwise two
 * alternating radii, then a wider arc. The first layout with every item inside the window, apart from each other and
 * from Mika's body is used.
 */
export function ringLayout(f: PetFrame, agents: number, music: boolean): RingSpots {
  const count = agents + 1 + (music ? 1 : 0);
  const { facing, span } = ringDirection(f);
  const quarter = Math.PI / 4;
  const split = (pts: Pt[], compact: boolean, radius: number, rings: number): RingSpots => ({
    agents: pts.slice(music ? 1 : 0, count - 1), music: music ? pts[0] : null, settings: pts[count - 1], compact, radius, rings,
  });
  let best: RingSpots | null = null, bestBad = Infinity;
  for (const wide of [span, Math.min(span * 1.5, Math.PI * 2), Math.min(span * 2, Math.PI * 2), Math.PI * 2]) {
    for (const turn of wide === span ? [0] : [0, quarter, -quarter]) {
      const dir = facing + turn;
      // [compact, rings, ring gap, first and last outer radius]
      for (const [compact, rings, gap, from, to] of [[false, 1, 0, 96, 134], [true, 1, 0, 96, 134], [true, 2, 38, 120, 134], [true, 1, 0, 136, 170], [true, 2, 38, 136, 190]] as const) {
        for (let outer = from; outer <= to; outer += 2) {
          const pts = arcPoints(f, count, dir, wide, outer, rings, gap);
          const bad = ringProblems(f, pts, compact);
          if (bad === 0) return split(pts, compact, outer, rings);
          if (bad < bestBad) { best = split(pts, compact, outer, rings); bestBad = bad; }
        }
      }
    }
  }
  return best!;
}

/** Use whichever side has more space after the native window is clamped to its display. */
export function speechBounds(f: PetFrame) {
  const margin = 6, gap = 6, half = 48;
  const side = f.cx > f.width - f.cx ? "left" : "right";
  const left = side === "left" ? margin : f.cx + half + gap;
  const end = side === "left" ? f.cx - half - gap : f.width - margin;
  return { side, left, width: Math.max(0, end - left) };
}

/** Clear space between her body and the chat panel, on whichever side the chat opens. */
export const CHAT_GAP = 8;

/**
 * The chat panel inside the window: the full width, from the window edge to CHAT_GAP short of Mika's body (not of her
 * square, whose empty margins used to leave 20 to 36 px of air). With Mika at the bottom the panel's bottom edge
 * is 28 px lower than before, at the top 12 px higher. The panel only ever overlaps her transparent margin.
 */
export function chatPanelRect(f: PetFrame): { left: number; top: number; width: number; height: number } {
  const boxTop = f.cy - PET_SIZE / 2;
  const below = f.cy > f.height / 2;
  const top = below ? 0 : Math.min(f.height, boxTop + BODY_BOTTOM + CHAT_GAP);
  const bottom = below ? Math.max(0, boxTop + BODY_TOP - CHAT_GAP) : f.height;
  return { left: 0, top, width: f.width, height: Math.max(0, bottom - top) };
}
