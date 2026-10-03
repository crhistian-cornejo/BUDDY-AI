// Small building blocks of the Settings page: sections with hairline cards, rows, switches and buttons.
// Text only goes in through textContent (see chat/dom.ts); nothing is parsed as HTML.
import { invoke } from "@tauri-apps/api/core";
import { h, svg } from "../chat/dom";
import { PROVIDER_MARKS } from "../chat/provider-marks";

type Child = Node | string | null | undefined | false;

/** A 24-grid Tabler outline icon. */
export function icon(path: string, size = 16): SVGElement {
  return svg(path, size, { fill: "none", stroke: "currentColor", "stroke-width": "1.75", "stroke-linecap": "round", "stroke-linejoin": "round" });
}

/** The page title and an optional line under it. */
export function header(title: string, detail?: string): HTMLElement {
  return h("header", { class: "page-header" }, h("h1", { text: title }), detail ? h("p", { class: "muted", text: detail }) : null);
}

/** A labelled group: a small heading over a hairline card. */
export function section(label: string | null, ...rows: Child[]): { el: HTMLElement; card: HTMLElement } {
  const card = h("div", { class: "card" }, ...rows);
  const el = h("section", { class: "group" }, label ? h("h2", { text: label }) : null, card);
  return { el, card };
}

/** One line of a card: what it is on the left, its control on the right. */
export function row(title: string | Node, detail: string | Node | null, ...controls: Child[]): HTMLElement {
  const text = h("div", { class: "row-text" },
    typeof title === "string" ? h("div", { class: "row-title", text: title }) : title,
    detail === null ? null : typeof detail === "string" ? h("div", { class: "row-detail", text: detail }) : detail);
  return h("div", { class: "row" }, text, controls.length ? h("div", { class: "row-controls" }, ...controls) : null);
}

/** A muted line inside a card when there is nothing to list. */
export function emptyRow(text: string, detail?: string): HTMLElement {
  return h("div", { class: "empty" }, h("div", { text }), detail ? h("div", { class: "muted small", text: detail }) : null);
}

/** A switch (role="switch"); `onChange` gets the new value. */
export function toggle(label: string, on: boolean, onChange: (on: boolean) => void): HTMLButtonElement {
  const el = h("button", { type: "button", class: "switch", role: "switch", "aria-checked": String(on), "aria-label": label },
    h("span", { class: "knob" }));
  el.addEventListener("click", () => {
    const next = el.getAttribute("aria-checked") !== "true";
    el.setAttribute("aria-checked", String(next));
    onChange(next);
  });
  return el;
}

/** A core on/off setting ("false" = off; anything else, missing included, = on) as a row with a switch. */
export function settingRow(key: string, title: string, detail: string): HTMLElement {
  const sw = toggle(title, true, (on) => void invoke("set_setting_flag", { key, on }));
  void invoke<boolean>("setting_flag", { key }).then((on) => sw.setAttribute("aria-checked", String(on)), () => {});
  const r = row(title, detail, sw);
  r.classList.add("clickable");
  r.querySelector(".row-text")!.addEventListener("click", () => sw.click());
  return r;
}

type Variant = "primary" | "secondary" | "ghost";

export function button(label: string, onClick: () => void, variant: Variant = "secondary", title?: string): HTMLButtonElement {
  const el = h("button", { type: "button", class: `btn ${variant}`, text: label, title });
  el.addEventListener("click", onClick);
  return el;
}

/** A square icon button; its name doubles as the tooltip. */
export function iconButton(path: string, title: string, onClick: () => void): HTMLButtonElement {
  const el = h("button", { type: "button", class: "icon-btn", title, "aria-label": title }, icon(path));
  el.addEventListener("click", onClick);
  return el;
}

/** Claude's, Codex's or Gemini's mark (Codex takes the text colour, so it shows on light and dark). */
export function providerMark(provider: string, size: number): HTMLElement {
  const mark = PROVIDER_MARKS[provider] ?? PROVIDER_MARKS.claude!;
  const el = h("span", { class: "mark", title: mark.label, style: mark.color ? `color:${mark.color}` : undefined });
  el.append(svg(mark.path, size, { fill: "currentColor" }));
  return el;
}

/** Token counts the way the Mac shows them: 950, 12.3 k, 1.2 M. */
export function k(n: number): string {
  return n >= 1_000_000 ? `${(n / 1_000_000).toFixed(1)} M` : n >= 1000 ? `${(n / 1000).toFixed(1)} k` : String(n);
}

export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}
