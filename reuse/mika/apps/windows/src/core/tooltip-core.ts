// Pure rules of the app-wide tooltip (no DOM, so node can test them): where the text comes from, when it shows,
// where it goes and how it is shortened. The DOM side is tooltip.ts. The macOS port follows the same numbers.

export const TIP_DELAY_MS = 450;
/** A second tooltip within this long of the previous one shows at once (like moving along a toolbar). */
export const TIP_WARM_MS = 600;
export const TIP_MARGIN = 6;
export const TIP_GAP = 6;
export const TIP_MAX_WIDTH = 240;
/** Nobody is looking for the text after this long; also drops a tooltip whose element was re-rendered away. */
export const TIP_AUTOHIDE_MS = 6000;

export interface Rect { left: number; top: number; width: number; height: number }
export interface Size { width: number; height: number }

export interface TipSource { text: string; key: string; adopt: "title" | null }

export interface TipAttrs { dataTip?: string | null; title?: string | null; ariaLabel?: string | null; tipKey?: string | null }

const clean = (s: string | null | undefined) => (s ?? "").replace(/[ \t\r\f\v]+/g, " ").trim();

/**
 * data-tip > title > aria-label. `adopt: "title"` tells the DOM side to move the title into data-tip (so the native
 * tooltip never shows). A title that is newer than an adopted data-tip is the caller's job (see tooltip.ts).
 */
export function tipSource(a: TipAttrs): TipSource | null {
  const key = clean(a.tipKey);
  const tip = clean(a.dataTip);
  if (tip) return { text: tip, key, adopt: null };
  const title = clean(a.title);
  if (title) return { text: title, key, adopt: "title" };
  const label = clean(a.ariaLabel);
  if (label) return { text: label, key, adopt: null };
  return null;
}

/** Milliseconds to wait before showing: none right after another tooltip, otherwise TIP_DELAY_MS. */
export function tipDelay(now: number, lastHiddenAt: number | null, visibleNow = false): number {
  if (visibleNow) return 0;
  if (lastHiddenAt != null && now - lastHiddenAt <= TIP_WARM_MS) return 0;
  return TIP_DELAY_MS;
}

/** The width the tooltip may use in this viewport. */
export const tipMaxWidth = (viewWidth: number) => Math.max(40, Math.min(TIP_MAX_WIDTH, viewWidth - TIP_MARGIN * 2));

export interface Placement { x: number; y: number; side: "below" | "above" | "over"; tailX: number }

/**
 * Below the element, centred; above when below has no room; clamped to the viewport with TIP_MARGIN. When neither
 * side has room for the whole height it overlaps the element (pointer-events none, so the element stays usable) at the
 * clamped position. `tailX` is the element's centre relative to the tooltip's left.
 */
export function placeTip(anchor: Rect, tip: Size, view: Size, margin = TIP_MARGIN, gap = TIP_GAP): Placement {
  const cx = anchor.left + anchor.width / 2;
  const maxX = Math.max(margin, view.width - margin - tip.width);
  const x = Math.min(Math.max(cx - tip.width / 2, margin), maxX);
  const roomBelow = view.height - margin - (anchor.top + anchor.height + gap);
  const roomAbove = anchor.top - gap - margin;
  let side: Placement["side"];
  let y: number;
  if (roomBelow >= tip.height) {
    side = "below";
    y = anchor.top + anchor.height + gap;
  } else if (roomAbove >= tip.height) {
    side = "above";
    y = anchor.top - gap - tip.height;
  } else {
    side = "over";
    const maxY = Math.max(margin, view.height - margin - tip.height);
    const wantY = roomBelow >= roomAbove ? anchor.top + anchor.height + gap : anchor.top - gap - tip.height;
    y = Math.min(Math.max(wantY, margin), maxY);
  }
  return { x, y, side, tailX: cx - x };
}

/**
 * The longest prefix of `text` (plus an ellipsis) for which `fits` is true; `text` itself when it fits.
 * `fits` measures the real element, so wrapping and font are honoured. Binary search: O(log n) measurements.
 */
export function fitText(text: string, fits: (candidate: string) => boolean): string {
  if (fits(text)) return text;
  const chars = [...text];
  let lo = 0;
  let hi = chars.length - 1;
  let best = "…";
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const candidate = chars.slice(0, mid).join("").trimEnd() + "…";
    if (fits(candidate)) {
      best = candidate;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return best;
}
