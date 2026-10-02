// How an assistant answer is drawn: markdown (bold, italic, lists, tables, code, quotes), LaTeX equations as MathML,
// a status line while the agent works, and the pages it used as small icons. Everything is built with DOM calls from
// the trees in core/markdown.ts and core/tex.ts: text only ever goes in through textContent, no HTML is parsed, and
// links are never clickable inside the answer (the sources row is the only way out, and it only opens http/https).
// Twin of Sources/Island/ChatMarkdownView.swift on macOS.

import "./answer.css";
import { h, svg } from "./dom";
import { Bridge, IS_TAURI } from "../core/bridge";
import { logoFor, fallbackColor, fallbackInitial, type Logo } from "../core/logos";
import { parseMarkdown, inlineSegments, cleanSources, taskMarker, matchLink, linkHost, linkKept, linkService, type ChatSource, type MdBlock, type MdItem } from "../core/markdown";
import { highlight } from "../core/highlight";
import { SERVICE_MARKS } from "./service-marks";
import { parseTex, SPACED_OPERATORS, type MathNode } from "../core/tex";

// ---- inline: bold, italic, code, strike, equations

/** Bold, italic, `code` and ~~strike~~ inside one run of text. An unclosed marker stays as written while it streams. */
function inlineNodes(src: string): Node[] {
  const out: Node[] = [];
  let buf = "";
  const flush = () => { if (buf) { out.push(document.createTextNode(buf)); buf = ""; } };
  const wrap = (tag: "strong" | "em" | "del", inner: string) => { flush(); out.push(h(tag, {}, ...inlineNodes(inner))); };
  let i = 0;
  while (i < src.length) {
    const c = src[i]!;
    if (c === "\\" && i + 1 < src.length && "\\`*_{}[]()#+-.!~|$".includes(src[i + 1]!)) { buf += src[i + 1]; i += 2; continue; }
    if (c === "`") {
      const end = src.indexOf("`", i + 1);
      if (end > i + 1) { flush(); out.push(h("code", { class: "md-code" }, src.slice(i + 1, end))); i = end + 1; continue; }
    }
    if (c === "[" || (c === "!" && src[i + 1] === "[")) {
      const link = matchLink(src, i);
      if (link) {
        flush();
        if (link.image) out.push(h("span", { class: "md-image" }, `🖼 ${link.label || "imagen"}`));
        else if (linkKept(link.url)) {
          // The user's own workspaces (a Doc, an event, a Meet room) open in the browser; any other address never does.
          const service = linkService(link.url);
          const mark = service ? SERVICE_MARKS[service] : null;
          out.push(h("a", { class: "md-link md-link-open", href: link.url, rel: "noopener noreferrer",
            onclick: (e: Event) => { e.preventDefault(); void Bridge.openUrl(link.url).catch(() => {}); } },
            mark ? h("span", { class: "md-link-icon", title: mark.label, style: `color:${mark.color}` }, svg(mark.path, 13, { fill: "currentColor" })) : null,
            ...inlineNodes(link.label || link.url)));
        } else {
          const host = linkHost(link.url);
          out.push(h("span", { class: "md-link" }, ...inlineNodes(link.label || link.url)));
          if (host) out.push(h("span", { class: "md-link-host" }, ` ${host}`));
        }
        i = link.end;
        continue;
      }
    }
    let matched = false;
    for (const [marker, tag] of [["**", "strong"], ["__", "strong"], ["~~", "del"]] as const) {
      if (!src.startsWith(marker, i)) continue;
      const end = src.indexOf(marker, i + 2);
      if (end > i + 2) { wrap(tag, src.slice(i + 2, end)); i = end + 2; matched = true; break; }
    }
    if (matched) continue;
    if ((c === "*" || c === "_") && src[i + 1] !== c && src[i + 1] !== " " && src[i + 1] !== undefined) {
      const intraword = c === "_" && /[\p{L}\p{N}]/u.test(src[i - 1] ?? "");
      if (!intraword) {
        let end = i + 1;
        while ((end = src.indexOf(c, end)) >= 0 && (src[end - 1] === " " || src[end + 1] === c || (c === "_" && /[\p{L}\p{N}]/u.test(src[end + 1] ?? "")))) end++;
        if (end > i + 1) { wrap("em", src.slice(i + 1, end)); i = end + 1; continue; }
      }
    }
    buf += c;
    i++;
  }
  flush();
  return out;
}

/** One paragraph of inline markdown, with its equations. */
export function inlineContent(src: string): Node[] {
  const out: Node[] = [];
  for (const seg of inlineSegments(src)) {
    if (seg.t === "text") { out.push(...inlineNodes(seg.s)); continue; }
    const node = parseTex(seg.s);
    out.push(node ? mathElement(node, false) : h("code", { class: "md-code" }, seg.s));
  }
  return out;
}

// ---- equations (MathML)

const MATHML = "http://www.w3.org/1998/Math/MathML";

function m(tag: string, attrs: Record<string, string> = {}, ...children: Array<Node | string>): Element {
  const el = document.createElementNS(MATHML, tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  for (const c of children) el.append(typeof c === "string" ? document.createTextNode(c) : c);
  return el;
}

const SPACE_WIDTH: Record<string, string> = { " ": "0.17em", " ": "0.22em", " ": "1em", " ": "0.28em", " ": "0.28em" };
const STACKED = new Set(["∑", "∏", "∐", "⋃", "⋂", "lim", "lim sup", "lim inf", "max", "min", "sup", "inf"]);
const ACCENT_MARK: Record<string, string> = { hat: "^", bar: "‾", vec: "→", tilde: "~", dot: "˙", ddot: "¨", overline: "‾", underline: "_" };

export function mathElement(node: MathNode, display: boolean): Element {
  const math = m("math", display ? { display: "block" } : {}, toMathML(node));
  math.setAttribute("class", display ? "md-math block" : "md-math");
  return math;
}

function toMathML(node: MathNode): Element {
  switch (node.t) {
    case "row": return m("mrow", {}, ...node.items.map(toMathML));
    case "sym": {
      const s = node.s;
      if (s in SPACE_WIDTH && [...s].length === 1 && /\s/.test(s)) return m("mspace", { width: SPACE_WIDTH[s]! });
      if (/^[0-9.]+$/.test(s)) return m("mn", {}, s);
      if (SPACED_OPERATORS.has(s) || (/^[\p{P}\p{S}]$/u.test(s) && !/\p{L}/u.test(s))) return m("mo", {}, s);
      return node.italic ? m("mi", {}, s) : m("mi", { mathvariant: "normal" }, s);
    }
    case "text": return m("mtext", {}, node.s);
    case "script": {
      const base = toMathML(node.base);
      if (node.sup && node.sub) return m("msubsup", {}, base, toMathML(node.sub), toMathML(node.sup));
      if (node.sup) return m("msup", {}, base, toMathML(node.sup));
      return m("msub", {}, base, toMathML(node.sub!));
    }
    case "frac": return m("mfrac", {}, toMathML(node.num), toMathML(node.den));
    case "sqrt": return node.index ? m("mroot", {}, toMathML(node.body), toMathML(node.index)) : m("msqrt", {}, toMathML(node.body));
    case "bigop": {
      const word = node.glyph.length > 1 || /\p{L}/u.test(node.glyph);
      const op = word ? m("mo", { movablelimits: "true" }, node.glyph) : m("mo", { largeop: "true", movablelimits: "true" }, node.glyph);
      const stacked = STACKED.has(node.glyph);
      if (node.sub && node.sup) return m(stacked ? "munderover" : "msubsup", {}, op, toMathML(node.sub), toMathML(node.sup));
      if (node.sub) return m(stacked ? "munder" : "msub", {}, op, toMathML(node.sub));
      if (node.sup) return m(stacked ? "mover" : "msup", {}, op, toMathML(node.sup));
      return op;
    }
    case "delim": return fenced(node.left, node.right, toMathML(node.body));
    case "matrix": {
      const align = node.align === "aligned" ? (node.left === "" && node.rows.some(r => r.length > 1) ? "first-right" : "left") : "center";
      const table = m("mtable", { class: `md-matrix ${align}` }, ...node.rows.map(row =>
        m("mtr", {}, ...row.map(cell => m("mtd", {}, toMathML(cell))))));
      return fenced(node.left, node.right, table);
    }
    case "accent": {
      const mark = m("mo", { stretchy: "false" }, ACCENT_MARK[node.kind]!);
      if (node.kind === "underline") return m("munder", { accentunder: "true" }, toMathML(node.body), mark);
      return m("mover", { accent: "true" }, toMathML(node.body), mark);
    }
    case "style": return m("mstyle", { mathvariant: node.kind === "bold" ? "bold" : "normal" }, toMathML(node.body));
  }
}

function fenced(left: string, right: string, body: Element): Element {
  const parts: Element[] = [];
  if (left) parts.push(m("mo", { fence: "true", stretchy: "true", form: "prefix" }, left));
  parts.push(body);
  if (right) parts.push(m("mo", { fence: "true", stretchy: "true", form: "postfix" }, right));
  return m("mrow", {}, ...parts);
}

// ---- blocks

function itemContent(item: MdItem): Node[] {
  const task = taskMarker(item.text);
  const out: Node[] = [];
  if (task) out.push(h("span", { class: `md-task${task.checked ? " done" : ""}`, "aria-hidden": "true" }, task.checked ? "☑" : "☐"));
  out.push(h("span", { class: "md-li-text" }, ...inlineContent(task ? task.rest : item.text)));
  if (item.children.length) out.push(h("div", { class: "md-li-children" }, ...item.children.map(blockElement)));
  return out;
}

/** A code block: a bar with the language and a Copy button, then the code in colour (plain where the language is unknown). */
function codeBlock(lang: string | null, code: string): HTMLElement {
  const copy = h("button", { class: "md-copy", type: "button", title: "Copiar el código" }, "Copiar");
  let reset: number | undefined;
  copy.addEventListener("click", () => {
    void navigator.clipboard?.writeText(code).then(() => {
      copy.textContent = "Copiado ✓";
      window.clearTimeout(reset);
      reset = window.setTimeout(() => { copy.textContent = "Copiar"; }, 1500);
    }).catch(() => { copy.textContent = "No se pudo"; });
  });
  const spans = highlight(code, lang).map(t => t.kind === "plain" ? document.createTextNode(t.text) : h("span", { class: `tk-${t.kind}` }, t.text));
  return h("div", { class: "md-codeblock" },
    h("div", { class: "md-codehead" }, h("span", { class: "md-lang" }, lang || "código"), copy),
    h("pre", {}, h("code", {}, ...spans)));
}

export function blockElement(block: MdBlock): HTMLElement {
  switch (block.t) {
    case "heading": return h("div", { class: `md-h md-h${Math.min(block.level, 4)}` }, ...inlineContent(block.text));
    case "paragraph": return h("p", { class: "md-p" }, ...inlineContent(block.text));
    case "bullet": return h("ul", { class: "md-ul" }, ...block.items.map(item => h("li", { class: taskMarker(item.text) ? "md-li-task" : "" }, ...itemContent(item))));
    case "ordered": return h("ol", { class: "md-ol", start: block.start }, ...block.items.map(item => h("li", { class: taskMarker(item.text) ? "md-li-task" : "" }, ...itemContent(item))));
    case "code": return codeBlock(block.lang, block.code);
    case "quote": return h("blockquote", { class: "md-quote" }, ...block.blocks.map(blockElement));
    case "rule": return h("hr", { class: "md-rule" });
    case "table": {
      const align = (i: number) => block.aligns[i] === "r" ? "right" : block.aligns[i] === "c" ? "center" : "left";
      const cell = (tag: "th" | "td", text: string, i: number) => h(tag, { style: `text-align:${align(i)}` }, ...inlineContent(text));
      return h("div", { class: "md-table" }, h("table", {},
        h("thead", {}, h("tr", {}, ...block.header.map((t, i) => cell("th", t, i)))),
        h("tbody", {}, ...block.rows.map(row => h("tr", {}, ...row.map((t, i) => cell("td", t, i)))))));
    }
    case "math": {
      const node = parseTex(block.latex);
      return h("div", { class: "md-mathblock", title: block.latex }, node ? mathElement(node, true) : h("code", { class: "md-code" }, block.latex));
    }
  }
}

/**
 * The markdown of one answer. `update` takes the whole text so far and only touches the blocks that changed: while
 * text streams in, only the block being written is built again.
 */
export class MarkdownView {
  readonly el = h("div", { class: "md" });
  private keys: string[] = [];

  update(text: string) {
    const blocks = parseMarkdown(text);
    for (let i = 0; i < blocks.length; i++) {
      const key = JSON.stringify(blocks[i]);
      if (this.keys[i] === key) continue;
      const next = blockElement(blocks[i]!);
      const old = this.el.children[i];
      if (old) old.replaceWith(next); else this.el.append(next);
      this.keys[i] = key;
    }
    while (this.el.children.length > blocks.length) this.el.lastElementChild!.remove();
    this.keys.length = blocks.length;
  }
}

// ---- sources and status

/** Site icons already asked for, by host: one request per site, whatever the number of answers that cite it. */
const icons = new Map<string, Promise<string | null>>();

function iconFor(source: ChatSource): Promise<string | null> {
  if (!IS_TAURI) return Promise.resolve(null);
  let icon = icons.get(source.host);
  if (!icon) {
    icon = Bridge.sourceIcon(source.url).then(url => (typeof url === "string" && url.startsWith("data:image/") ? url : null)).catch(() => null);
    icons.set(source.host, icon);
  }
  return icon;
}

const SVG_NS = "http://www.w3.org/2000/svg";
const svgEl = (name: string, attrs: Record<string, string>): SVGElement => {
  const el = document.createElementNS(SVG_NS, name);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  return el;
};

/** A drawn logo as an <svg> built node by node (no markup is parsed). */
function logoSvg(logo: Logo): SVGElement {
  const el = svgEl("svg", { viewBox: "0 0 24 24", width: "20", height: "20", "aria-hidden": "true", focusable: "false" });
  el.append(svgEl("rect", { width: "24", height: "24", fill: logo.bg }));
  for (const sh of logo.shapes) {
    el.append(svgEl("path", sh.stroke
      ? { d: sh.d, fill: "none", stroke: sh.stroke, "stroke-width": String(sh.w ?? 1.8), "stroke-linecap": "round", "stroke-linejoin": "round" }
      : { d: sh.d, fill: sh.fill ?? "#fff" }));
  }
  return el;
}

/**
 * A page the answer used: the site's logo. Well-known sites have a mark drawn in code (instant, no request); any
 * other site shows a letter tile and, unless the user turned it off, its own icon once Rust has fetched it. The
 * tooltip names the page and the site; a click opens it.
 */
function sourceChip(source: ChatSource, depth: number): HTMLElement {
  const logo = logoFor(source.host);
  const tile = logo
    ? h("i", { class: "has-logo", "data-logo": logo.id }, logoSvg(logo))
    : h("i", { style: `background:${fallbackColor(source.host)}` }, fallbackInitial(source.host));
  if (!logo) void iconFor(source).then(url => {
    if (!url) return;
    tile.textContent = "";
    tile.classList.add("has-icon");
    tile.append(h("img", { src: url, alt: "", draggable: "false" }));
  });
  return h("button", { class: "src-chip", type: "button", style: `z-index:${depth}`, title: `${source.title || source.host}
${source.host}`,
    "aria-label": `Abrir ${source.title || source.host}`, onclick: () => { void Bridge.openUrl(source.url).catch(() => {}); } }, tile);
}

/**
 * The sources as a stack: the newest (the last page fetched or searched) on top with its site's name beside it; the
 * stack spreads out on hover so every site can be told apart and clicked.
 */
export function sourcesRow(sources: ChatSource[]): HTMLElement | null {
  if (!sources.length) return null;
  const limit = 8;
  const shown = sources.slice(-limit);
  const hiddenCount = sources.length - shown.length;
  const stack = h("div", { class: "src-stack" }, ...shown.map((s, i) => sourceChip(s, i + 1)));
  const last = shown[shown.length - 1]!;
  return h("div", { class: "src-row" },
    hiddenCount > 0 ? h("em", { title: sources.slice(0, hiddenCount).map(s => s.host).join("\n") }, `+${hiddenCount}`) : null,
    stack,
    h("span", { class: "src-last", text: last.host }));
}

/** One assistant message: status line, markdown and sources. Used for finished answers and, updated, for the live one. */
export class AnswerView {
  readonly el = h("div", { class: "answer" });
  private readonly statusEl = h("div", { class: "answer-status", role: "status" });
  private readonly md = new MarkdownView();
  private sourcesEl: HTMLElement | null = null;
  private sourcesKey = "";

  constructor() {
    this.statusEl.hidden = true;
    this.el.append(this.statusEl, this.md.el);
  }

  update(text: string, opts: { status?: string | null; sources?: ChatSource[] } = {}) {
    const cleaned = cleanSources(text);
    this.md.update(cleaned.text);
    this.md.el.hidden = !cleaned.text;
    const status = opts.status ?? null;
    this.statusEl.hidden = !status;
    if (status && this.statusEl.textContent !== status) this.statusEl.textContent = status;
    const all = [...(opts.sources ?? [])];
    for (const s of cleaned.sources) if (!all.some(x => x.url === s.url)) all.push(s);
    const key = all.map(s => s.url).join("\n");
    if (key !== this.sourcesKey) {
      this.sourcesKey = key;
      this.sourcesEl?.remove();
      this.sourcesEl = sourcesRow(all);
      if (this.sourcesEl) this.el.append(this.sourcesEl);
    }
  }
}
