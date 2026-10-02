// App-wide tooltip: ONE set of delegated listeners on the document, one element, no per-control wiring and no timers
// while nothing is shown. Text comes from `data-tip`, else the control's `title` (moved into `data-tip` on first hover so
// the native tooltip never appears; `aria-label` is kept/added for screen readers), else `aria-label` on buttons.
// `data-tip-key="Ctrl K"` adds a keycap. Text is only ever set through textContent.

import "./tooltip.css";
import { TIP_AUTOHIDE_MS, fitText, placeTip, tipDelay, tipMaxWidth, tipSource } from "./tooltip-core";

/** aria-label alone only becomes a tooltip on things you can press (a labelled section or input would be noise). */
const PRESSABLE = 'button, a[href], summary, [role="button"], [role="tab"], [role="menuitem"], input[type="range"]';
const CANDIDATE = "[data-tip], [title], [aria-label]";

let installed = false;
let suppress: () => boolean = () => false;
let el: HTMLDivElement | null = null;
let label: HTMLSpanElement;
let keycap: HTMLSpanElement;
let host: HTMLElement | null = null;
let showTimer = 0;
let hideTimer = 0;
let lastHiddenAt: number | null = null;
let visible = false;

function ensureEl(): HTMLDivElement {
  if (el) return el;
  const tip = document.createElement("div");
  tip.className = "mika-tip";
  tip.setAttribute("role", "tooltip");
  tip.setAttribute("aria-hidden", "true"); // the control's own aria-label is what a screen reader reads
  label = document.createElement("span");
  keycap = document.createElement("span");
  keycap.className = "mika-tip-key";
  tip.append(label, keycap);
  document.body.append(tip);
  el = tip;
  return tip;
}

/** Finds the element a pointer/focus target belongs to and settles its tooltip text; null when it has none. */
function resolve(target: EventTarget | null): { node: HTMLElement; text: string; key: string } | null {
  if (!(target instanceof Element)) return null;
  let node: HTMLElement | null = target.closest<HTMLElement>(CANDIDATE);
  while (node) {
    if (node.getAttribute("aria-hidden") !== "true" && !node.matches(":disabled")) {
      const found = settle(node);
      // The mascot's hover ring already names each agent under its icon: a tooltip on top of those names only gets in
      // the way. The title is still adopted above, so the native tooltip does not come back.
      if (node.closest("#pet-ring")) return null;
      // Text the control already shows is not worth repeating (a ring item labelled "Música" with title "Música").
      const own = (node.textContent ?? "").replace(/\s+/g, " ").trim().toLowerCase();
      if (found && !(own && found.text.toLowerCase() === own)) return { node, text: found.text, key: found.key };
    }
    node = node.parentElement?.closest<HTMLElement>(CANDIDATE) ?? null;
  }
  return null;
}

function settle(node: HTMLElement): { text: string; key: string } | null {
  // A title written after we adopted the old one wins (code updates `.title` on live controls); an empty one clears it.
  if (node.hasAttribute("data-tip-auto") && node.hasAttribute("title")) {
    const fresh = node.getAttribute("title") ?? "";
    node.removeAttribute("title");
    if (fresh.trim()) {
      node.setAttribute("data-tip", fresh);
      if (node.hasAttribute("data-tip-aria")) node.setAttribute("aria-label", fresh);
    } else {
      node.removeAttribute("data-tip");
      node.removeAttribute("data-tip-auto");
      if (node.hasAttribute("data-tip-aria")) {
        node.removeAttribute("aria-label");
        node.removeAttribute("data-tip-aria");
      }
    }
  }
  const src = tipSource({
    dataTip: node.getAttribute("data-tip"),
    title: node.getAttribute("title"),
    ariaLabel: node.matches(PRESSABLE) ? node.getAttribute("aria-label") : null,
    tipKey: node.getAttribute("data-tip-key"),
  });
  if (!src) return null;
  if (src.adopt === "title") {
    const raw = node.getAttribute("title") ?? "";
    node.removeAttribute("title");
    node.setAttribute("data-tip", raw);
    node.setAttribute("data-tip-auto", "");
    if (!node.getAttribute("aria-label")) {
      node.setAttribute("aria-label", src.text);
      node.setAttribute("data-tip-aria", "");
    }
  }
  return { text: src.text, key: src.key };
}

function cancelTimers() {
  if (showTimer) { clearTimeout(showTimer); showTimer = 0; }
  if (hideTimer) { clearTimeout(hideTimer); hideTimer = 0; }
}

function hide() {
  cancelTimers();
  host = null;
  if (visible) {
    visible = false;
    lastHiddenAt = performance.now();
    el?.classList.remove("on");
  }
}

function show(node: HTMLElement, text: string, key: string) {
  showTimer = 0;
  if (!node.isConnected || suppress()) return;
  const tip = ensureEl();
  const view = { width: document.documentElement.clientWidth, height: document.documentElement.clientHeight };
  tip.style.maxWidth = `${tipMaxWidth(view.width)}px`;
  keycap.textContent = key;
  label.textContent = text;
  // Measure while invisible (visibility:hidden still lays out).
  tip.style.left = "0px";
  tip.style.top = "0px";
  const limitH = view.height - 12;
  if (tip.offsetHeight > limitH) {
    label.textContent = fitText(text, (c) => { label.textContent = c; return tip.offsetHeight <= limitH; });
  }
  const r = node.getBoundingClientRect();
  const p = placeTip(
    { left: r.left, top: r.top, width: r.width, height: r.height },
    { width: tip.offsetWidth, height: tip.offsetHeight },
    view,
  );
  tip.style.left = `${Math.round(p.x)}px`;
  tip.style.top = `${Math.round(p.y)}px`;
  host = node;
  visible = true;
  tip.classList.add("on");
  if (hideTimer) clearTimeout(hideTimer);
  hideTimer = window.setTimeout(hide, TIP_AUTOHIDE_MS);
}

function arm(target: EventTarget | null, warmable = true) {
  const hit = resolve(target);
  if (!hit) { hide(); return; }
  if (hit.node === host) return; // already showing, or already waiting for, this one
  const wasVisible = visible;
  hide();
  const delay = tipDelay(performance.now(), warmable ? lastHiddenAt : null, wasVisible);
  host = hit.node;
  if (delay === 0) show(hit.node, hit.text, hit.key);
  else showTimer = window.setTimeout(() => show(hit.node, hit.text, hit.key), delay);
}

/**
 * Installs the delegated listeners once per window. `shouldSuppress` lets a window veto tooltips while its host is
 * hidden or collapsed (the island's wake strip).
 */
export function installTooltips(shouldSuppress?: () => boolean) {
  if (shouldSuppress) suppress = shouldSuppress;
  if (installed || typeof document === "undefined") return;
  installed = true;
  const opts = { capture: true, passive: true } as const;
  document.addEventListener("pointerover", (e) => {
    if (e.pointerType === "touch" || e.buttons !== 0) { hide(); return; } // a drag, or no hover at all
    arm(e.target);
  }, opts);
  document.addEventListener("pointerout", (e) => {
    if (!e.relatedTarget) hide(); // left the window
    else if (host && e.target instanceof Node && host.contains(e.target) && !host.contains(e.relatedTarget as Node)) hide();
  }, opts);
  document.addEventListener("focusin", (e) => {
    if (e.target instanceof Element && e.target.matches(":focus-visible")) arm(e.target, false);
  }, opts);
  document.addEventListener("focusout", hide, opts);
  document.addEventListener("pointerdown", hide, opts);
  document.addEventListener("keydown", (e) => { if (e.key === "Escape" && (visible || showTimer)) hide(); }, opts);
  document.addEventListener("scroll", () => { if (visible || showTimer) hide(); }, opts);
  window.addEventListener("blur", hide);
  document.addEventListener("visibilitychange", () => { if (document.hidden) hide(); });
}
