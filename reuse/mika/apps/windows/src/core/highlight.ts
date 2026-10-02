// Syntax colouring for the code blocks of an answer. Pure, so it runs under `node --test`. Twin of
// apps/macos/Sources/Providers/Markdown/CodeHighlighter.swift: the same languages, the same scanner, the same token kinds.
//
// It is a scanner, not a parser: it knows comments, strings, numbers, a short keyword list per language and little else,
// which is enough to make code readable and cheap enough to run on every delta while an answer streams. The tokens always
// cover the whole input, in order, so `tokens.map(t => t.text).join("") === code` for any input (even broken code).
// Unknown languages and very long blocks come back as one plain token.

export type TokenKind = "kw" | "str" | "com" | "num" | "type" | "fn" | "prop" | "tag" | "add" | "del" | "hunk" | "plain";
export interface Token { kind: TokenKind; text: string }

/** Longer than this is shown plain: a pasted log should not cost a scan per streamed word. */
export const MAX_HIGHLIGHT = 30_000;

interface Cfg {
  kw: Set<string>;
  line: string[];
  block?: [string, string];
  triple?: string[];
  quotes: string[];
  /** A backtick string may span lines (JavaScript template literals, Go raw strings). */
  template?: boolean;
  /** `'a'` is a character only when it closes right away; otherwise the quote is plain (Rust lifetimes). */
  charQuote?: boolean;
  ci?: boolean;
  /** An identifier or string followed by `:` is a key (JSON, YAML, CSS). */
  keyColon?: boolean;
  /** `-` can be part of a name (CSS properties, YAML keys). */
  dash?: boolean;
  /** `$name` and `${name}` are variables. */
  dollar?: boolean;
  /** `@name` is an attribute / decorator. */
  at?: boolean;
  /** `#fff` is a colour. */
  hexColor?: boolean;
  /** `# comment` only at the start of a word (shell). */
  hashWord?: boolean;
}

const words = (s: string) => new Set(s.split(/\s+/).filter(Boolean));

const C_FAMILY = { line: ["//"], block: ["/*", "*/"] as [string, string], quotes: ['"', "'"] };

const JS_KW = words("var let const function return if else for while do switch case break continue default new delete typeof instanceof in of this super class extends import export from as async await yield try catch finally throw void null undefined true false static get set interface type enum implements public private protected readonly declare namespace abstract keyof satisfies is");
const SWIFT_KW = words("let var func class struct enum protocol extension import return if else guard switch case default for while repeat break continue in is as try catch throw throws rethrows do defer init deinit self Self super nil true false static public private fileprivate internal open final lazy weak unowned mutating nonmutating override some any async await actor where associatedtype typealias subscript inout operator indirect convenience required");
const PY_KW = words("def class return if elif else for while break continue pass import from as try except finally raise with lambda yield global nonlocal assert del in is not and or None True False async await match case self cls");
const RUST_KW = words("fn let mut const static struct enum impl trait pub use mod crate self Self super match if else for while loop break continue return as in where type unsafe async await move ref dyn box true false extern macro_rules");
const GO_KW = words("func var const type struct interface map chan package import return if else for range switch case default break continue go defer select fallthrough goto nil true false iota");
const JAVA_KW = words("class interface enum extends implements import package return if else for while do switch case default break continue new this super try catch finally throw throws public private protected static final abstract void int long short byte float double boolean char null true false instanceof synchronized volatile transient native var record sealed permits fun val when object companion data override open internal lateinit by is in out sealed suspend");
const C_KW = words("auto break case char const continue default do double else enum extern float for goto if inline int long register return short signed sizeof static struct switch typedef union unsigned void volatile while class namespace template typename public private protected virtual new delete this nullptr true false bool using try catch throw operator constexpr override final noexcept include define ifdef ifndef endif pragma string var foreach in is as null abstract sealed readonly lock async await");
const SH_KW = words("if then else elif fi for while until do done case esac function in select time return exit break continue export local readonly declare unset source alias echo cd set shift trap eval exec");
const SQL_KW = words("select from where and or not null is in like between join inner outer left right full cross on as group by order having limit offset insert into values update set delete create table alter drop index view primary key foreign references unique default check constraint distinct union all exists case when then else end asc desc count sum avg min max with returning begin commit rollback true false cascade if truncate");
const YAML_KW = words("true false null yes no on off");
const JSON_KW = words("true false null");
const CSS_KW = words("important inherit initial unset auto none");
const PHP_KW = words("function class public private protected static return if else elseif foreach for while do switch case break continue new echo print null true false array use namespace extends implements interface trait try catch finally throw const abstract final isset unset empty require include require_once include_once as fn match");

const CONFIGS: Record<string, Cfg> = {
  js: { ...C_FAMILY, quotes: ['"', "'", "`"], kw: JS_KW, template: true, at: true },
  swift: { ...C_FAMILY, kw: SWIFT_KW, at: true },
  python: { kw: PY_KW, line: ["#"], quotes: ['"', "'"], triple: ['"""', "'''"], at: true },
  rust: { ...C_FAMILY, kw: RUST_KW, charQuote: true },
  go: { ...C_FAMILY, quotes: ['"', "'", "`"], kw: GO_KW, template: true },
  java: { ...C_FAMILY, kw: JAVA_KW, at: true },
  c: { ...C_FAMILY, kw: C_KW },
  shell: { kw: SH_KW, line: ["#"], quotes: ['"', "'", "`"], dollar: true, hashWord: true },
  sql: { kw: SQL_KW, line: ["--"], block: ["/*", "*/"], quotes: ["'", '"'], ci: true },
  yaml: { kw: YAML_KW, line: ["#"], quotes: ['"', "'"], keyColon: true, dash: true, hashWord: true },
  json: { kw: JSON_KW, line: [], quotes: ['"'], keyColon: true },
  css: { kw: CSS_KW, line: [], block: ["/*", "*/"], quotes: ['"', "'"], keyColon: true, dash: true, at: true, hexColor: true },
  php: { ...C_FAMILY, kw: PHP_KW, line: ["//", "#"], dollar: true },
};

const ALIASES: Record<string, string> = {
  js: "js", jsx: "js", javascript: "js", mjs: "js", cjs: "js", ts: "js", tsx: "js", typescript: "js", node: "js",
  swift: "swift", py: "python", python: "python", python3: "python", rust: "rust", rs: "rust", go: "go", golang: "go",
  java: "java", kotlin: "java", kt: "java", scala: "java", dart: "java", c: "c", h: "c", cpp: "c", "c++": "c", cc: "c", hpp: "c",
  csharp: "c", cs: "c", "c#": "c", objc: "c", bash: "shell", sh: "shell", zsh: "shell", shell: "shell", console: "shell", fish: "shell",
  sql: "sql", postgres: "sql", postgresql: "sql", mysql: "sql", sqlite: "sql", yaml: "yaml", yml: "yaml", toml: "yaml",
  json: "json", jsonc: "json", json5: "json", css: "css", scss: "css", less: "css",
  html: "html", xml: "html", svg: "html", xhtml: "html", vue: "html", plist: "html", php: "php", diff: "diff", patch: "diff",
};

/** The language family a fence's info string names (`ts`, `TypeScript`, `bash title="x"`), or null when it is unknown. */
export function languageOf(info: string | null | undefined): string | null {
  const first = (info ?? "").trim().split(/[\s{,]/)[0]?.toLowerCase().replace(/^language-/, "") ?? "";
  return ALIASES[first] ?? null;
}

const isIdentStart = (c: string) => /[A-Za-z_$]/.test(c) || c.charCodeAt(0) > 127;
const isIdentPart = (c: string) => /[A-Za-z0-9_$]/.test(c) || c.charCodeAt(0) > 127;
const isDigit = (c: string) => c >= "0" && c <= "9";

/** The end of the number that starts at `i` (digits, `_`, one `.`, an exponent, a hex / binary prefix and a unit suffix). */
function numberEnd(s: string, i: number): number {
  let j = i;
  if (s[j] === "0" && /[xXbBoO]/.test(s[j + 1] ?? "")) { j += 2; while (j < s.length && /[0-9a-fA-F_]/.test(s[j])) j++; return j; }
  while (j < s.length && (isDigit(s[j]) || s[j] === "_")) j++;
  if (s[j] === "." && isDigit(s[j + 1] ?? "")) { j++; while (j < s.length && (isDigit(s[j]) || s[j] === "_")) j++; }
  if (/[eE]/.test(s[j] ?? "") && (isDigit(s[j + 1] ?? "") || (/[+-]/.test(s[j + 1] ?? "") && isDigit(s[j + 2] ?? "")))) {
    j += 2; while (j < s.length && isDigit(s[j])) j++;
  }
  while (j < s.length && /[A-Za-z%]/.test(s[j])) j++;                       // 12px, 3.5f, 100ms, 10u32
  return j;
}

function push(out: Token[], kind: TokenKind, text: string) {
  if (!text) return;
  const last = out[out.length - 1];
  if (last && last.kind === kind) last.text += text; else out.push({ kind, text });
}

function nextNonSpace(s: string, from: number): string {
  let j = from;
  while (j < s.length && (s[j] === " " || s[j] === "\t")) j++;
  return s[j] ?? "";
}

function scanGeneric(s: string, cfg: Cfg): Token[] {
  const out: Token[] = [];
  let i = 0;
  while (i < s.length) {
    const c = s[i];
    // comments
    const lineStart = cfg.line.find((p) => s.startsWith(p, i) && (!cfg.hashWord || p !== "#" || i === 0 || /\s/.test(s[i - 1])));
    if (lineStart) {
      let j = s.indexOf("\n", i);
      if (j < 0) j = s.length;
      push(out, "com", s.slice(i, j)); i = j; continue;
    }
    if (cfg.block && s.startsWith(cfg.block[0], i)) {
      const end = s.indexOf(cfg.block[1], i + cfg.block[0].length);
      const j = end < 0 ? s.length : end + cfg.block[1].length;
      push(out, "com", s.slice(i, j)); i = j; continue;
    }
    // strings
    const triple = cfg.triple?.find((q) => s.startsWith(q, i));
    if (triple) {
      const end = s.indexOf(triple, i + 3);
      const j = end < 0 ? s.length : end + 3;
      push(out, "str", s.slice(i, j)); i = j; continue;
    }
    if (cfg.quotes.includes(c)) {
      if (c === "'" && cfg.charQuote) {
        const m = /^'(\\.|[^\\'\n])'/.exec(s.slice(i, i + 8));
        if (!m) { push(out, "plain", c); i++; continue; }
      }
      const multiline = c === "`" && cfg.template;
      let j = i + 1;
      while (j < s.length && s[j] !== c) {
        if (s[j] === "\\" && j + 1 < s.length) j++;
        else if (s[j] === "\n" && !multiline) break;
        j++;
      }
      if (s[j] === c) j++;
      const key = cfg.keyColon && nextNonSpace(s, j) === ":";
      push(out, key ? "prop" : "str", s.slice(i, j)); i = j; continue;
    }
    // numbers and colours
    if (isDigit(c) || (c === "." && isDigit(s[i + 1] ?? "") && !isIdentPart(s[i - 1] ?? " "))) {
      const j = numberEnd(s, i);
      push(out, "num", s.slice(i, j)); i = j; continue;
    }
    if (cfg.hexColor && c === "#" && /[0-9a-fA-F]/.test(s[i + 1] ?? "")) {
      let j = i + 1;
      while (j < s.length && /[0-9a-fA-F]/.test(s[j])) j++;
      push(out, "num", s.slice(i, j)); i = j; continue;
    }
    // variables, attributes
    if (cfg.dollar && c === "$" && (isIdentStart(s[i + 1] ?? " ") || s[i + 1] === "{")) {
      let j = i + 1;
      if (s[j] === "{") { const close = s.indexOf("}", j); j = close < 0 ? s.length : close + 1; }
      else while (j < s.length && isIdentPart(s[j])) j++;
      push(out, "prop", s.slice(i, j)); i = j; continue;
    }
    if (cfg.at && c === "@" && isIdentStart(s[i + 1] ?? " ")) {
      let j = i + 1;
      while (j < s.length && (isIdentPart(s[j]) || (cfg.dash && s[j] === "-"))) j++;
      push(out, "kw", s.slice(i, j)); i = j; continue;
    }
    // words
    if (isIdentStart(c)) {
      let j = i + 1;
      while (j < s.length && (isIdentPart(s[j]) || (cfg.dash && s[j] === "-" && isIdentPart(s[j + 1] ?? " ")))) j++;
      const word = s.slice(i, j);
      const lookup = cfg.ci ? word.toLowerCase() : word;
      const after = s[j] ?? "";
      let kind: TokenKind = "plain";
      if (cfg.kw.has(lookup)) kind = "kw";
      else if (cfg.keyColon && nextNonSpace(s, j) === ":" && s[j + 1] !== ":") kind = "prop";
      else if (after === "(") kind = "fn";
      else if (/^[A-Z]/.test(word) && (word.length === 1 || /[a-z]/.test(word))) kind = "type";
      push(out, kind, word); i = j; continue;
    }
    push(out, "plain", c); i++;
  }
  return out;
}

function scanHtml(s: string): Token[] {
  const out: Token[] = [];
  let i = 0;
  while (i < s.length) {
    if (s.startsWith("<!--", i)) {
      const end = s.indexOf("-->", i + 4);
      const j = end < 0 ? s.length : end + 3;
      push(out, "com", s.slice(i, j)); i = j; continue;
    }
    if (s[i] === "<" && /[A-Za-z/!?]/.test(s[i + 1] ?? "")) {
      let j = i + 1;
      while (j < s.length && /[/!?A-Za-z0-9:_.-]/.test(s[j])) j++;
      push(out, "tag", s.slice(i, j)); i = j;
      // attributes up to the closing `>`
      while (i < s.length && s[i] !== ">") {
        const c = s[i];
        if (c === '"' || c === "'") {
          let k = i + 1;
          while (k < s.length && s[k] !== c) k++;
          if (s[k] === c) k++;
          push(out, "str", s.slice(i, k)); i = k;
        } else if (/[A-Za-z_:@]/.test(c)) {
          let k = i + 1;
          while (k < s.length && /[A-Za-z0-9_:.@-]/.test(s[k])) k++;
          push(out, "prop", s.slice(i, k)); i = k;
        } else if (c === "/" && s[i + 1] === ">") { break; }
        else { push(out, "plain", c); i++; }
      }
      if (s[i] === "/" && s[i + 1] === ">") { push(out, "tag", "/>"); i += 2; }
      else if (s[i] === ">") { push(out, "tag", ">"); i++; }
      continue;
    }
    push(out, "plain", s[i]); i++;
  }
  return out;
}

function scanDiff(s: string): Token[] {
  const out: Token[] = [];
  for (const line of s.split(/(?<=\n)/)) {
    const kind: TokenKind = line.startsWith("+++") || line.startsWith("---") ? "hunk"
      : line.startsWith("+") ? "add" : line.startsWith("-") ? "del" : line.startsWith("@@") ? "hunk" : "plain";
    push(out, kind, line);
  }
  return out;
}

/** `code` as coloured spans. `language` is the fence's info string. */
export function highlight(code: string, language?: string | null): Token[] {
  const family = languageOf(language);
  if (!family || code.length > MAX_HIGHLIGHT) return code ? [{ kind: "plain", text: code }] : [];
  if (family === "html") return scanHtml(code);
  if (family === "diff") return scanDiff(code);
  return scanGeneric(code, CONFIGS[family]);
}
