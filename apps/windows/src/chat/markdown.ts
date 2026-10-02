// Ported from MIKA (MIT, © MIKA contributors, revision d050bc5): apps/windows/src/core/markdown.ts
// A small, forgiving markdown reader for chat answers: only a tree of blocks, no HTML is ever interpreted. The
// text of a paragraph or list item stays as source and is styled later (bold, italic, code and inline maths).
// It must cope with half-written answers because it runs while the text is streaming.
// Pure, so it runs under `node --test`. Twin of Sources/Providers/Markdown/MarkdownParser.swift and
// MarkdownSources.swift on macOS: tests/markdown.test.mjs holds the same cases.

export interface MdItem { text: string; children: MdBlock[] }
export type MdAlign = "l" | "c" | "r";
export type MdBlock =
  | { t: "heading"; level: number; text: string }
  | { t: "paragraph"; text: string }
  | { t: "bullet"; items: MdItem[] }
  | { t: "ordered"; start: number; items: MdItem[] }
  | { t: "code"; lang: string | null; code: string }
  | { t: "quote"; blocks: MdBlock[] }
  | { t: "rule" }
  | { t: "table"; header: string[]; aligns: MdAlign[]; rows: string[][] }
  | { t: "math"; latex: string };

export function parseMarkdown(source: string): MdBlock[] {
  return blocks(source.replace(/\r\n/g, "\n").split("\n"));
}

const trim = (s: string) => s.replace(/^[ \t]+|[ \t]+$/g, "");

function blocks(lines: string[]): MdBlock[] {
  const result: MdBlock[] = [];
  let paragraph: string[] = [];
  let i = 0;
  const flush = () => { if (paragraph.length) { result.push({ t: "paragraph", text: paragraph.join("\n") }); paragraph = []; } };

  while (i < lines.length) {
    const line = lines[i]!;
    const trimmed = trim(line);
    if (!trimmed) { flush(); i++; continue; }

    const fence = trimmed.startsWith("```") ? "```" : trimmed.startsWith("~~~") ? "~~~" : null;
    if (fence) {
      flush();
      const info = trim(trimmed.slice(3));
      const code: string[] = [];
      i++;
      while (i < lines.length && !trim(lines[i]!).startsWith(fence)) { code.push(lines[i]!); i++; }
      i++;                                                  // the closing fence (or the end of the text)
      result.push({ t: "code", lang: info.split(" ")[0] || null, code: code.join("\n") });
      continue;
    }

    const math = displayMath(lines, i);
    if (math) { flush(); result.push({ t: "math", latex: math.latex }); i = math.next; continue; }

    const heading = /^(#{1,6}) (.*)$/.exec(trimmed);
    if (heading) {
      flush();
      result.push({ t: "heading", level: heading[1]!.length, text: trim(heading[2]!.replace(/#+\s*$/, "")) });
      i++; continue;
    }
    if (isRule(trimmed)) { flush(); result.push({ t: "rule" }); i++; continue; }

    if (trimmed.startsWith(">")) {
      flush();
      const quoted: string[] = [];
      while (i < lines.length && trim(lines[i]!).startsWith(">")) {
        let content = trim(lines[i]!).slice(1);
        if (content.startsWith(" ")) content = content.slice(1);
        quoted.push(content);
        i++;
      }
      result.push({ t: "quote", blocks: blocks(quoted) });
      continue;
    }

    const list = parseList(lines, i);
    if (list) { flush(); result.push(list.block); i = list.next; continue; }
    const table = parseTable(lines, i);
    if (table) { flush(); result.push(table.block); i = table.next; continue; }

    paragraph.push(trimmed);
    i++;
  }
  flush();
  return result;
}

/** `$$ … $$` or `\[ … \]`, on one line or across several. Never closed → null (it stays text until it is). */
function displayMath(lines: string[], start: number): { latex: string; next: number } | null {
  const trimmed = trim(lines[start]!);
  for (const [open, close] of [["$$", "$$"], ["\\[", "\\]"]] as const) {
    if (!trimmed.startsWith(open)) continue;
    const rest = trimmed.slice(open.length);
    const end = rest.indexOf(close);
    if (end >= 0) {
      const after = trim(rest.slice(end + close.length));
      return after ? null : { latex: rest.slice(0, end).trim(), next: start + 1 };
    }
    const content = trim(rest) ? [rest] : [];
    for (let j = start + 1; j < lines.length; j++) {
      const at = lines[j]!.indexOf(close);
      if (at >= 0) {
        content.push(lines[j]!.slice(0, at));
        return { latex: content.join("\n").trim(), next: j + 1 };
      }
      content.push(lines[j]!);
    }
    return null;
  }
  return null;
}

function isRule(trimmed: string): boolean {
  const chars = trimmed.replace(/ /g, "");
  return chars.length >= 3 && "-*_".includes(chars[0]!) && [...chars].every(c => c === chars[0]);
}

// ---- lists

interface Marker { indent: number; ordered: boolean; number: number; text: string }

const indentOf = (line: string) => /^[ \t]*/.exec(line)![0].length;

function listMarker(line: string): Marker | null {
  const indent = indentOf(line);
  const rest = line.slice(indent);
  if (rest.length >= 2 && "-*+".includes(rest[0]!) && rest[1] === " ") return { indent, ordered: false, number: 0, text: rest.slice(2) };
  const m = /^(\d{1,9})([.)]) (.*)$/.exec(rest);
  if (m) return { indent, ordered: true, number: Number(m[1]), text: m[3]! };
  return null;
}

function parseList(lines: string[], start: number): { block: MdBlock; next: number } | null {
  const first = listMarker(lines[start]!);
  if (!first) return null;
  const items: MdItem[] = [];
  let i = start;

  while (i < lines.length) {
    const marker = listMarker(lines[i]!);
    if (!marker || marker.indent !== first.indent || marker.ordered !== first.ordered) break;
    const item: MdItem = { text: marker.text, children: [] };
    i++;
    const inner: string[] = [];
    while (i < lines.length) {
      const current = lines[i]!;
      if (!trim(current)) {
        let j = i + 1;
        while (j < lines.length && !trim(lines[j]!)) j++;
        if (j < lines.length && indentOf(lines[j]!) > first.indent) { inner.push(""); i++; continue; }
        break;
      }
      if (indentOf(current) > first.indent) {
        inner.push(current.slice(Math.min(indentOf(current), first.indent + 2)));
        i++;
        continue;
      }
      break;
    }
    if (inner.length) item.children = blocks(inner);
    items.push(item);

    // a blank line between two items of the same list does not end it
    let j = i;
    while (j < lines.length && !trim(lines[j]!)) j++;
    if (j > i && j < lines.length) {
      const next = listMarker(lines[j]!);
      if (next && next.indent === first.indent && next.ordered === first.ordered) i = j;
    }
  }
  return { block: first.ordered ? { t: "ordered", start: first.number, items } : { t: "bullet", items }, next: i };
}

// ---- tables

function cells(row: string): string[] {
  let t = trim(row);
  if (t.startsWith("|")) t = t.slice(1);
  if (t.endsWith("|")) t = t.slice(0, -1);
  return t.split("|").map(trim);
}

function aligns(separator: string): MdAlign[] | null {
  if (!separator.includes("-")) return null;
  const result: MdAlign[] = [];
  for (const part of cells(separator)) {
    const core = part.replace(/^:+|:+$/g, "");
    if (!core || ![...core].every(c => c === "-")) return null;
    const left = part.startsWith(":"), right = part.endsWith(":");
    result.push(left && right ? "c" : right ? "r" : "l");
  }
  return result.length ? result : null;
}

function parseTable(lines: string[], start: number): { block: MdBlock; next: number } | null {
  if (start + 1 >= lines.length || !lines[start]!.includes("|")) return null;
  const a = aligns(lines[start + 1]!);
  if (!a) return null;
  const rows: string[][] = [];
  let i = start + 2;
  while (i < lines.length && lines[i]!.includes("|") && trim(lines[i]!)) { rows.push(cells(lines[i]!)); i++; }
  return { block: { t: "table", header: cells(lines[start]!), aligns: a, rows }, next: i };
}

// ---- inline maths

export type InlineSegment = { t: "text"; s: string } | { t: "math"; s: string };

/**
 * Splits a line of text into plain markdown text and inline formulas (`$x$`, `$$x$$`, `\(x\)`). Money ("$5 and $10"),
 * code spans, escaped dollars and unclosed or multi-line dollars are left as text.
 */
export function inlineSegments(source: string): InlineSegment[] {
  const chars = [...source];
  const out: InlineSegment[] = [];
  let buffer = "";
  let i = 0;
  const flush = () => { if (buffer) { out.push({ t: "text", s: buffer }); buffer = ""; } };
  const find = (pattern: string, from: number): number => {
    const p = [...pattern];
    for (let k = from; k + p.length <= chars.length; k++) if (p.every((ch, n) => chars[k + n] === ch)) return k;
    return -1;
  };

  while (i < chars.length) {
    const c = chars[i]!;
    if (c === "\\" && i + 1 < chars.length) {
      if (chars[i + 1] === "(") {
        const end = find("\\)", i + 2);
        if (end >= 0) { flush(); out.push({ t: "math", s: chars.slice(i + 2, end).join("").trim() }); i = end + 2; continue; }
      }
      buffer += c + chars[i + 1]; i += 2;                   // keep escapes for the markdown pass
      continue;
    }
    if (c === "`") {
      const end = chars.indexOf("`", i + 1);
      if (end >= 0) { buffer += chars.slice(i, end + 1).join(""); i = end + 1; continue; }   // code span: untouched
    }
    if (c === "$") {
      if (chars[i + 1] === "$") {
        const end = find("$$", i + 2);
        if (end > i + 2) { flush(); out.push({ t: "math", s: chars.slice(i + 2, end).join("").trim() }); i = end + 2; continue; }
      }
      const end = closingDollar(chars, i + 1);
      if (end >= 0) { flush(); out.push({ t: "math", s: chars.slice(i + 1, end).join("") }); i = end + 1; continue; }
    }
    buffer += c;
    i++;
  }
  flush();
  return out;
}

/** A closing `$` needs a formula that does not start or end with a space, stays on one line and is not followed by a digit. */
function closingDollar(chars: string[], start: number): number {
  if (start >= chars.length || /\s/.test(chars[start]!) || chars[start] === "$") return -1;
  let j = start;
  while (j < chars.length) {
    const c = chars[j]!;
    if (c === "\n") return -1;
    if (c === "\\") { j += 2; continue; }
    if (c === "$" && !/\s/.test(chars[j - 1]!) && (j + 1 >= chars.length || !/[0-9]/.test(chars[j + 1]!))) return j;
    j++;
  }
  return -1;
}

// ---- sources

/** A page an answer used, shown as a small icon under the answer (never as a link in the text). */
export interface ChatSource { title: string; url: string; host: string }

export function hostOf(url: string): string | null {
  try {
    const host = new URL(url).hostname.toLowerCase();
    return host ? host.replace(/^www\./, "") : null;
  } catch { return null; }
}

/** Only web addresses are ever sources: anything else (`javascript:`, `file:`…) is refused. */
export function makeSource(title: string, url: string): ChatSource | null {
  const address = url.trim();
  if (!/^https?:\/\/[^\s/]+/i.test(address)) return null;
  const host = hostOf(address);
  if (!host) return null;
  const name = title.trim();
  return { title: name || host, url: address, host };
}

const GENERIC_LABELS = new Set(["fuente", "fuentes", "source", "sources", "aquí", "aqui", "link", "enlace", "ver más", "ver mas", "leer más",
  "leer mas", "más información", "mas informacion", "here", "read more", "click aquí", "haz clic aquí"]);
const LABEL_LINE = /^(?:#{1,6}\s*)?(?:\*\*|__)?\s*(?:fuentes?|sources?|referencias?|enlaces|links)\s*:?\s*(?:\*\*|__)?\s*:?\s*$/i;
const ONLY_LINK = /^(?:[-*+•]|\d+[.)])?\s*(?:\[([^\]]*)\]\(((?:[^()\s]|\([^()\s]*\))+)\)|<?(https?:\/\/[^\s<>]+)>?)\s*$/;

function stripTrailingPunctuation(url: string): string {
  return url.replace(/[.,;:!?"']+$/, "");
}

/**
 * An answer from the web is full of links. They are shown as small icons under the answer, so the text carries no
 * addresses: a link keeps its label (or disappears when it is only a citation) and a bare address shows only its
 * site. Everything taken out is returned as a source. Code is never touched.
 */
/**
 * Which links stay in the text, clickable. An answer is untrusted text, so by default no address is clickable (they go to
 * the sources row, which opens http/https only). The exceptions are the user's own workspaces, where a connector hands
 * files, events and meetings back from: a Google Doc, a Calendar event, a Meet room. Exact hosts, https only, no
 * credentials in the address, no other port. Twin of `LinkPolicy.keeps` in MarkdownSources.swift on macOS.
 */
const KEPT_HOSTS = new Set([
  "docs.google.com", "drive.google.com", "sheets.google.com", "slides.google.com", "forms.google.com", "meet.google.com",
  "mail.google.com", "calendar.google.com", "contacts.google.com", "github.com", "notion.so", "www.notion.so",
]);

export type LinkServiceId = "docs" | "sheets" | "slides" | "forms" | "drive" | "calendar" | "meet" | "gmail" | "github" | "notion";

/** The service a kept link belongs to (a Doc, a Sheet, a Calendar event, a Meet room…), or null when it has no icon. */
export function linkService(raw: string): LinkServiceId | null {
  if (!linkKept(raw)) return null;
  const url = new URL(raw.trim());
  const host = url.hostname.toLowerCase(), path = url.pathname;
  switch (host) {
    case "docs.google.com":
      if (path.startsWith("/spreadsheets")) return "sheets";
      if (path.startsWith("/presentation")) return "slides";
      if (path.startsWith("/forms")) return "forms";
      return "docs";
    case "sheets.google.com": return "sheets";
    case "slides.google.com": return "slides";
    case "forms.google.com": return "forms";
    case "drive.google.com": return "drive";
    case "calendar.google.com": case "www.google.com": return "calendar";
    case "meet.google.com": return "meet";
    case "mail.google.com": return "gmail";
    case "github.com": return "github";
    case "notion.so": case "www.notion.so": return "notion";
    default: return null;
  }
}

export function linkKept(raw: string): boolean {
  let url: URL;
  try { url = new URL(raw.trim()); } catch { return false; }
  if (url.protocol !== "https:" || url.username || url.password || url.port) return false;
  const host = url.hostname.toLowerCase();
  return KEPT_HOSTS.has(host) || (host === "www.google.com" && url.pathname.startsWith("/calendar/"));
}

export function cleanSources(source: string): { text: string; sources: ChatSource[] } {
  const sources: ChatSource[] = [];
  const out: string[] = [];
  let changed = false;
  let fence: string | null = null;
  const add = (s: ChatSource) => { if (!sources.some(x => x.url === s.url)) sources.push(s); };

  for (const line of source.replace(/\r\n/g, "\n").split("\n")) {
    const trimmed = trim(line);
    if (fence) {
      if (trimmed.startsWith(fence)) fence = null;
      out.push(line); continue;
    }
    if (trimmed.startsWith("```")) { fence = "```"; out.push(line); continue; }
    if (trimmed.startsWith("~~~")) { fence = "~~~"; out.push(line); continue; }

    const only = ONLY_LINK.exec(trimmed);
    if (only) {
      const address = only[2] ?? (only[3] ? stripTrailingPunctuation(only[3]) : "");
      // A line that is only a workspace link is the answer itself, not a citation.
      if (!(address && linkKept(address))) {
        const s = only[2] ? makeSource(only[1] ?? "", only[2]) : only[3] ? makeSource("", stripTrailingPunctuation(only[3])) : null;
        if (s) { add(s); changed = true; continue; }
      }
    }
    const r = inline(line);
    r.sources.forEach(add);
    if (r.changed) changed = true;
    out.push(r.text);
  }

  // A "Fuentes:" label left at the end has nothing under it any more.
  let trimmedTail = false;
  while (out.length) {
    const last = out[out.length - 1]!;
    const blank = !trim(last);
    if (!(blank || LABEL_LINE.test(trim(last)))) break;
    if (blank && !changed) break;
    out.pop(); trimmedTail = true;
  }
  if (!changed && !trimmedTail) return { text: source, sources: [] };
  return { text: out.join("\n"), sources };
}

function inline(line: string): { text: string; sources: ChatSource[]; changed: boolean } {
  const chars = [...line];
  const out: string[] = [];
  const sources: ChatSource[] = [];
  let changed = false;
  let i = 0;
  const startsWith = (text: string, at: number) => chars.slice(at, at + text.length).join("") === text;

  while (i < chars.length) {
    const c = chars[i]!;

    if (c === "`") {
      const end = chars.indexOf("`", i + 1);
      if (end >= 0) { out.push(...chars.slice(i, end + 1)); i = end + 1; continue; }     // code span: untouched
    }

    if (c === "[" && out[out.length - 1] !== "!") {
      const link = parseLink(chars, i);
      if (!link && isOpenLink(chars, i)) { out.push(...chars.slice(i)); break; }            // still streaming: wait
      if (link) {
        // The user's own workspace links stay in the text as links.
        if (linkKept(link.url)) { out.push(...chars.slice(i, link.end)); i = link.end; continue; }
        changed = true;
        const source = makeSource(link.label, link.url);
        if (source) {
          sources.push(source);
          const label = link.label.trim();
          const citation = GENERIC_LABELS.has(label.toLowerCase()) || !label || /^\d+$/.test(label);
          const wrapped = out[out.length - 1] === "(" && link.end < chars.length && chars[link.end] === ")";
          if (wrapped || citation) {
            if (wrapped) { out.pop(); i = link.end + 1; } else { i = link.end; }
            const next = i < chars.length ? chars[i]! : null;
            if (out[out.length - 1] === " " && (next === null || ".,;:!?)".includes(next) || wrapped)) out.pop();
            continue;
          }
          out.push(...label);
        } else {
          out.push(...link.label);                                                          // unsafe scheme: label only
        }
        i = link.end;
        continue;
      }
    }

    const prev = out[out.length - 1];
    const wordBefore = prev !== undefined && /[\p{L}\p{N}]/u.test(prev);
    const bracketed = c === "<" && (startsWith("<http://", i) || startsWith("<https://", i));
    if (!wordBefore && (bracketed || startsWith("http://", i) || startsWith("https://", i))) {
      let j = bracketed ? i + 1 : i;
      const from = j;
      while (j < chars.length && !/\s/.test(chars[j]!) && !"<>)]\"'".includes(chars[j]!)) j++;
      const url = stripTrailingPunctuation(chars.slice(from, j).join(""));
      if (linkKept(url)) {
        changed = true;
        out.push(...`[${new URL(url).hostname}](${url})`);
        i = from + [...url].length;
        if (bracketed && chars[i] === ">") i++;
        continue;
      }
      const source = makeSource("", url);
      if (source && source.host.includes(".")) {
        changed = true;
        sources.push(source);
        out.push(...source.host);
        i = from + [...url].length;
        if (bracketed && chars[i] === ">") i++;
        continue;
      }
    }

    out.push(c);
    i++;
  }
  return { text: out.join(""), sources, changed };
}

/** `[label](` with no closing parenthesis yet. */
function isOpenLink(chars: string[], start: number): boolean {
  let i = start + 1;
  while (i < chars.length && chars[i] !== "]") {
    if (chars[i] === "[" || chars[i] === "\n") return false;
    i++;
  }
  return i + 1 < chars.length && chars[i] === "]" && chars[i + 1] === "(";
}

/** `[label](address)` starting at `start`. A link that is not complete yet (still streaming) is not a link. */
function parseLink(chars: string[], start: number): { label: string; url: string; end: number } | null {
  let i = start + 1;
  const label: string[] = [];
  while (i < chars.length && chars[i] !== "]") {
    if (chars[i] === "[" || chars[i] === "\n") return null;
    label.push(chars[i]!); i++;
  }
  if (!(i + 1 < chars.length && chars[i] === "]" && chars[i + 1] === "(")) return null;
  i += 2;
  const url: string[] = [];
  let depth = 0;
  while (i < chars.length) {
    const c = chars[i]!;
    if (c === "(") depth++;
    else if (c === ")") {
      if (depth === 0) return { label: label.join(""), url: url.join("").trim(), end: i + 1 };
      depth--;
    } else if (c === "\n") return null;
    url.push(c); i++;
  }
  return null;
}

// ---- small helpers the renderers share (twin of MarkdownInline in MarkdownParser.swift)

/** A task-list item: `[ ] algo` / `[x] algo`. */
export function taskMarker(text: string): { checked: boolean; rest: string } | null {
  const m = /^\[( |x|X)\][ \t]+(.*)$/s.exec(text);
  return m ? { checked: m[1] !== " ", rest: m[2]! } : null;
}

export interface InlineLink { label: string; url: string; image: boolean; end: number }

/**
 * `[label](url)` or `![alt](url)` starting at `i`, or null. The url is whatever sits between the parentheses (no nested
 * parentheses, no spaces); `end` is the index after the closing `)`. Nothing is interpreted: the renderers show the label
 * and the site, and never make it clickable.
 */
export function matchLink(src: string, i: number): InlineLink | null {
  const image = src[i] === "!";
  const open = image ? i + 1 : i;
  if (src[open] !== "[") return null;
  let depth = 0, close = -1;
  for (let j = open; j < src.length; j++) {
    if (src[j] === "\\") { j++; continue; }
    if (src[j] === "[") depth++;
    else if (src[j] === "]" && --depth === 0) { close = j; break; }
  }
  if (close < 0 || src[close + 1] !== "(") return null;
  const end = src.indexOf(")", close + 2);
  if (end < 0) return null;
  const url = src.slice(close + 2, end).trim().split(/\s+/)[0] ?? "";
  if (!url || /[<>]/.test(url)) return null;
  return { label: src.slice(open + 1, close), url, image, end: end + 1 };
}

/** The site a link points to, for the small note beside its label; null for anything but http(s). */
export function linkHost(url: string): string | null {
  return /^https?:\/\//i.test(url) ? hostOf(url) : null;
}
