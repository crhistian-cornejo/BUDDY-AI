// Ported from MIKA (MIT, © MIKA contributors, revision d050bc5): apps/windows/src/core/tex.ts
// LaTeX maths → a small tree, with no library. The chat draws the tree as MathML (the browser engine lays out
// fractions, roots, sums and matrices itself). Anything this parser does not understand makes `parseTex` return null,
// so the caller shows the source instead of a wrong picture. Pure, so it runs under `node --test`.
// Twin of Sources/Providers/Markdown/MathParser.swift on macOS: tests/tex.test.mjs holds the same cases.

export type MathAccent = "hat" | "bar" | "vec" | "tilde" | "dot" | "ddot" | "overline" | "underline";
export type MathNode =
  | { t: "row"; items: MathNode[] }
  | { t: "sym"; s: string; italic: boolean }
  | { t: "text"; s: string }
  | { t: "script"; base: MathNode; sup: MathNode | null; sub: MathNode | null }
  | { t: "frac"; num: MathNode; den: MathNode }
  | { t: "sqrt"; body: MathNode; index: MathNode | null }
  | { t: "bigop"; glyph: string; sub: MathNode | null; sup: MathNode | null }
  | { t: "delim"; left: string; right: string; body: MathNode }
  | { t: "matrix"; rows: MathNode[][]; left: string; right: string; align: "center" | "aligned" }
  | { t: "accent"; kind: MathAccent; body: MathNode }
  | { t: "style"; kind: "bold" | "roman"; body: MathNode };

class Fail extends Error {}

export function parseTex(latex: string): MathNode | null {
  const scanner = new Scanner([...latex]);
  try {
    const node = scanner.parseRow();
    if (!scanner.atEnd) return null;
    if (node.t === "row" && node.items.length === 0) return null;
    return node;
  } catch (e) {
    if (e instanceof Fail) return null;
    throw e;
  }
}

// ---- tables

const GREEK: Record<string, [string, boolean]> = {
  alpha: ["α", true], beta: ["β", true], gamma: ["γ", true], delta: ["δ", true], epsilon: ["ϵ", true], varepsilon: ["ε", true],
  zeta: ["ζ", true], eta: ["η", true], theta: ["θ", true], vartheta: ["ϑ", true], iota: ["ι", true], kappa: ["κ", true],
  lambda: ["λ", true], mu: ["μ", true], nu: ["ν", true], xi: ["ξ", true], omicron: ["ο", true], pi: ["π", true], varpi: ["ϖ", true],
  rho: ["ρ", true], varrho: ["ϱ", true], sigma: ["σ", true], varsigma: ["ς", true], tau: ["τ", true], upsilon: ["υ", true],
  phi: ["ϕ", true], varphi: ["φ", true], chi: ["χ", true], psi: ["ψ", true], omega: ["ω", true],
  Gamma: ["Γ", false], Delta: ["Δ", false], Theta: ["Θ", false], Lambda: ["Λ", false], Xi: ["Ξ", false], Pi: ["Π", false],
  Sigma: ["Σ", false], Upsilon: ["Υ", false], Phi: ["Φ", false], Psi: ["Ψ", false], Omega: ["Ω", false],
};

const SYMBOLS: Record<string, string> = {
  pm: "±", mp: "∓", times: "×", cdot: "⋅", div: "÷", leq: "≤", le: "≤", geq: "≥", ge: "≥", neq: "≠", ne: "≠", approx: "≈",
  equiv: "≡", sim: "∼", simeq: "≃", cong: "≅", propto: "∝", infty: "∞", partial: "∂", nabla: "∇", to: "→", rightarrow: "→",
  leftarrow: "←", gets: "←", leftrightarrow: "↔", Rightarrow: "⇒", Leftarrow: "⇐", Leftrightarrow: "⇔", implies: "⇒", iff: "⇔",
  mapsto: "↦", uparrow: "↑", downarrow: "↓", in: "∈", notin: "∉", ni: "∋", subset: "⊂", subseteq: "⊆", supset: "⊃", supseteq: "⊇",
  cup: "∪", cap: "∩", emptyset: "∅", varnothing: "∅", forall: "∀", exists: "∃", neg: "¬", lnot: "¬", land: "∧", wedge: "∧",
  lor: "∨", vee: "∨", ldots: "…", dots: "…", cdots: "⋯", vdots: "⋮", ddots: "⋱", circ: "∘", bullet: "∙", star: "⋆", angle: "∠",
  perp: "⊥", parallel: "∥", degree: "°", prime: "′", ell: "ℓ", hbar: "ℏ", Re: "ℜ", Im: "ℑ", aleph: "ℵ", oplus: "⊕", otimes: "⊗",
  ll: "≪", gg: "≫", mid: "∣", setminus: "∖", langle: "⟨", rangle: "⟩", lfloor: "⌊", rfloor: "⌋", lceil: "⌈", rceil: "⌉",
  "%": "%", "&": "&", "_": "_", "#": "#", "$": "$", "{": "{", "}": "}", "|": "‖", vert: "|", Vert: "‖",
};

/** Symbols that read better with a space on each side. */
export const SPACED_OPERATORS = new Set(["+", "−", "=", "<", ">", "≤", "≥", "≠", "≈", "≡", "∼", "≃", "≅", "∝", "±", "∓", "×", "⋅", "÷",
  "→", "←", "↔", "⇒", "⇐", "⇔", "↦", "∈", "∉", "∋", "⊂", "⊆", "⊃", "⊇", "∪", "∩", "∧", "∨", "∘", "∣", "∥", "≪", "≫", "⊕", "⊗", "∖"]);

const SPACES: Record<string, string> = { ",": " ", ";": " ", ":": " ", " ": " ", quad: " ", qquad: "  " };

/** Operators that carry limits. */
const BIG_OPERATORS: Record<string, string> = {
  sum: "∑", prod: "∏", coprod: "∐", bigcup: "⋃", bigcap: "⋂", int: "∫", iint: "∬", iiint: "∭", oint: "∮",
  lim: "lim", limsup: "lim sup", liminf: "lim inf", max: "max", min: "min", sup: "sup", inf: "inf",
};

const FUNCTIONS = new Set(["sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh", "coth",
  "log", "ln", "lg", "exp", "det", "dim", "ker", "gcd", "deg", "arg", "Pr", "hom", "mod"]);

const DOUBLE_STRUCK: Record<string, string> = { R: "ℝ", N: "ℕ", Z: "ℤ", Q: "ℚ", C: "ℂ", P: "ℙ", H: "ℍ", E: "𝔼" };
const CALLIGRAPHIC: Record<string, string> = {
  L: "ℒ", H: "ℋ", F: "ℱ", E: "ℰ", B: "ℬ", I: "ℐ", M: "ℳ", R: "ℛ", N: "𝒩", O: "𝒪", P: "𝒫", A: "𝒜", C: "𝒞", D: "𝒟", G: "𝒢",
  J: "𝒥", K: "𝒦", S: "𝒮", T: "𝒯", U: "𝒰", V: "𝒱", W: "𝒲", X: "𝒳", Y: "𝒴", Z: "𝒵", Q: "𝒬",
};

const ACCENTS: Record<string, MathAccent> = {
  hat: "hat", widehat: "hat", bar: "bar", vec: "vec", overrightarrow: "vec", tilde: "tilde", widetilde: "tilde",
  dot: "dot", ddot: "ddot", overline: "overline", underline: "underline",
};

const ENVIRONMENTS: Record<string, { left: string; right: string; align: "center" | "aligned" }> = {
  matrix: { left: "", right: "", align: "center" }, pmatrix: { left: "(", right: ")", align: "center" },
  bmatrix: { left: "[", right: "]", align: "center" }, Bmatrix: { left: "{", right: "}", align: "center" },
  vmatrix: { left: "|", right: "|", align: "center" }, Vmatrix: { left: "‖", right: "‖", align: "center" },
  cases: { left: "{", right: "", align: "aligned" }, array: { left: "", right: "", align: "center" },
  aligned: { left: "", right: "", align: "aligned" }, align: { left: "", right: "", align: "aligned" },
  "align*": { left: "", right: "", align: "aligned" }, split: { left: "", right: "", align: "aligned" },
  gathered: { left: "", right: "", align: "center" }, equation: { left: "", right: "", align: "center" },
  "equation*": { left: "", right: "", align: "center" },
};

const DELIMITER_SYMBOLS = new Set(["{", "}", "⟨", "⟩", "⌊", "⌋", "⌈", "⌉", "‖", "|"]);

// ---- scanner

const isLetter = (c: string) => /\p{L}/u.test(c);
const isDigit = (c: string) => c >= "0" && c <= "9";

function glyph(c: string): string {
  switch (c) {
    case "-": return "−";
    case "*": return "∗";
    case "'": return "′";
    case "~": return " ";
    default: return c;
  }
}

class Scanner {
  pos = 0;
  private readonly chars: string[];
  constructor(chars: string[]) { this.chars = chars; }

  get atEnd() { return this.pos >= this.chars.length; }

  private skipSpaces() { while (this.pos < this.chars.length && /\s/.test(this.chars[this.pos]!)) this.pos++; }

  private get atLineBreak() { return this.chars[this.pos] === "\\" && this.chars[this.pos + 1] === "\\"; }

  /** `\name` or `\<symbol>` at the cursor, without consuming it. */
  private peekCommand(): string | null {
    if (this.chars[this.pos] !== "\\") return null;
    let p = this.pos + 1;
    if (p >= this.chars.length) return "";
    if (isLetter(this.chars[p]!)) {
      let name = "";
      while (p < this.chars.length && isLetter(this.chars[p]!)) name += this.chars[p++];
      return name;
    }
    return this.chars[p]!;
  }

  private readCommand(): string {
    const name = this.peekCommand();
    if (name === null) return "";
    this.pos += 1 + Math.max([...name].length, 1);
    if (name === "") this.pos = this.chars.length;
    return name;
  }

  parseRow(): MathNode {
    const items: MathNode[] = [];
    for (;;) {
      this.skipSpaces();
      if (this.atEnd) break;
      const c = this.chars[this.pos]!;
      if (c === "}" || c === "&" || this.atLineBreak) break;
      const cmd = this.peekCommand();
      if (cmd === "right" || cmd === "end") break;
      items.push(this.parseScripts(this.parseAtom()));
    }
    return items.length === 1 ? items[0]! : { t: "row", items };
  }

  private parseScripts(base: MathNode): MathNode {
    let sup: MathNode | null = null, sub: MathNode | null = null;
    for (;;) {
      this.skipSpaces();
      if (this.atEnd) break;
      const c = this.chars[this.pos];
      if (c === "^") { this.pos++; if (sup) throw new Fail(); sup = this.parseArgument(); }
      else if (c === "_") { this.pos++; if (sub) throw new Fail(); sub = this.parseArgument(); }
      else break;
    }
    if (!sup && !sub) return base;
    if (base.t === "bigop" && !base.sub && !base.sup) return { t: "bigop", glyph: base.glyph, sub, sup };
    return { t: "script", base, sup, sub };
  }

  /** `{group}` or one single token (a letter, a digit, a symbol or one command). */
  private parseArgument(): MathNode {
    this.skipSpaces();
    if (this.atEnd) throw new Fail();
    const c = this.chars[this.pos]!;
    if (c === "{") return this.parseGroup();
    if (c === "\\") return this.parseCommand();
    if (c === "}" || c === "^" || c === "_" || c === "&") throw new Fail();
    this.pos++;
    return isLetter(c) ? { t: "sym", s: c, italic: true } : { t: "sym", s: glyph(c), italic: false };
  }

  private parseGroup(): MathNode {
    this.pos++;                                              // {
    const inner = this.parseRow();
    if (this.chars[this.pos] !== "}") throw new Fail();
    this.pos++;
    return inner;
  }

  private parseAtom(): MathNode {
    this.skipSpaces();
    if (this.atEnd) throw new Fail();
    const c = this.chars[this.pos]!;
    switch (c) {
      case "{": return this.parseGroup();
      case "\\": return this.parseCommand();
      case "^": case "_": return { t: "row", items: [] };    // a script with no base: the caller reads the script
      case "}": case "&": throw new Fail();
    }
    if (isDigit(c)) {
      let digits = "";
      while (this.pos < this.chars.length) {
        const d = this.chars[this.pos]!;
        if (isDigit(d)) { digits += d; this.pos++; }
        else if (d === "." && digits && isDigit(this.chars[this.pos + 1] ?? "")) { digits += d; this.pos++; }
        else break;
      }
      return { t: "sym", s: digits, italic: false };
    }
    this.pos++;
    return isLetter(c) ? { t: "sym", s: c, italic: true } : { t: "sym", s: glyph(c), italic: false };
  }

  private parseCommand(): MathNode {
    const name = this.readCommand();
    const greek = GREEK[name];
    if (greek) return { t: "sym", s: greek[0], italic: greek[1] };
    if (name in SPACES) return { t: "sym", s: SPACES[name]!, italic: false };
    if (name === "!") return { t: "row", items: [] };
    if (name in BIG_OPERATORS) return { t: "bigop", glyph: BIG_OPERATORS[name]!, sub: null, sup: null };
    if (FUNCTIONS.has(name)) return { t: "sym", s: name, italic: false };
    if (name in ACCENTS) return { t: "accent", kind: ACCENTS[name]!, body: this.parseArgument() };
    if (name in SYMBOLS) return { t: "sym", s: SYMBOLS[name]!, italic: false };

    switch (name) {
      case "frac": case "dfrac": case "tfrac": {
        const num = this.parseArgument();
        return { t: "frac", num, den: this.parseArgument() };
      }
      case "binom": case "dbinom": case "tbinom": {
        const top = this.parseArgument();
        return { t: "matrix", rows: [[top], [this.parseArgument()]], left: "(", right: ")", align: "center" };
      }
      case "sqrt": {
        this.skipSpaces();
        let index: MathNode | null = null;
        if (this.chars[this.pos] === "[") {
          const close = this.chars.indexOf("]", this.pos);
          if (close < 0) throw new Fail();
          const inner = new Scanner(this.chars.slice(this.pos + 1, close));
          index = inner.parseRow();
          if (!inner.atEnd) throw new Fail();
          this.pos = close + 1;
        }
        return { t: "sqrt", body: this.parseArgument(), index };
      }
      case "left": {
        const left = this.parseDelimiter();
        const body = this.parseRow();
        if (this.peekCommand() !== "right") throw new Fail();
        this.readCommand();
        return { t: "delim", left, right: this.parseDelimiter(), body };
      }
      case "text": case "textrm": case "mbox": case "textit": case "textsf":
        return { t: "text", s: this.readBraced() };
      case "mathrm": case "operatorname": case "mathsf": case "mathtt":
        return { t: "style", kind: "roman", body: this.parseArgument() };
      case "textbf": case "mathbf": case "boldsymbol": case "bm": case "pmb":
        return { t: "style", kind: "bold", body: this.parseArgument() };
      case "mathit":
        return this.parseArgument();
      case "mathbb": {
        const arg = this.parseArgument();
        if (arg.t === "sym" && DOUBLE_STRUCK[arg.s]) return { t: "sym", s: DOUBLE_STRUCK[arg.s]!, italic: false };
        return arg;
      }
      case "mathcal": case "mathscr": {
        const arg = this.parseArgument();
        if (arg.t === "sym" && CALLIGRAPHIC[arg.s]) return { t: "sym", s: CALLIGRAPHIC[arg.s]!, italic: false };
        return arg;
      }
      case "begin": return this.parseEnvironment();
      default: throw new Fail();                             // includes a stray \right or \end, and unknown commands
    }
  }

  /** The delimiter after `\left` / `\right`: `.` means none. */
  private parseDelimiter(): string {
    this.skipSpaces();
    if (this.atEnd) throw new Fail();
    const c = this.chars[this.pos]!;
    if (c === ".") { this.pos++; return ""; }
    if (c === "\\") {
      const name = this.readCommand();
      const symbol = SYMBOLS[name];
      if (symbol && DELIMITER_SYMBOLS.has(symbol)) return symbol;
      throw new Fail();
    }
    if (!"()[]|/<>".includes(c)) throw new Fail();
    this.pos++;
    return c;
  }

  /** Raw text between braces (nested braces allowed), for `\text{…}` and environment names. */
  private readBraced(): string {
    this.skipSpaces();
    if (this.chars[this.pos] !== "{") throw new Fail();
    let depth = 0, out = "";
    while (this.pos < this.chars.length) {
      const c = this.chars[this.pos++]!;
      if (c === "{") { depth++; if (depth === 1) continue; }
      if (c === "}") { depth--; if (depth === 0) return out; }
      out += c;
    }
    throw new Fail();
  }

  private parseEnvironment(): MathNode {
    const environment = this.readBraced();
    const layout = ENVIRONMENTS[environment];
    if (!layout) throw new Fail();
    if (environment === "array") {
      this.skipSpaces();
      if (this.chars[this.pos] === "{") this.readBraced();   // the column specification
    }
    const rows: MathNode[][] = [[]];
    for (;;) {
      rows[rows.length - 1]!.push(this.parseRow());
      this.skipSpaces();
      if (this.atEnd) throw new Fail();
      if (this.chars[this.pos] === "&") { this.pos++; continue; }
      if (this.atLineBreak) { this.pos += 2; rows.push([]); continue; }
      if (this.peekCommand() === "end") {
        this.readCommand();
        if (this.readBraced() !== environment) throw new Fail();
        break;
      }
      throw new Fail();
    }
    const last = rows[rows.length - 1]!;
    if (last.length === 1 && last[0]!.t === "row" && (last[0] as { items: MathNode[] }).items.length === 0) rows.pop();   // a trailing \\
    if (!rows.length) throw new Fail();
    return { t: "matrix", rows, left: layout.left, right: layout.right, align: layout.align };
  }
}
