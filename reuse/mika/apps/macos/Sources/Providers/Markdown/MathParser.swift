import Foundation

// LaTeX maths → a small tree, with no library. The tree is drawn by `MathView` (stacked fractions, exponents, roots,
// sums, matrices…) and `plainText` gives a Unicode version for formulas inside a line of text. Anything this parser does
// not understand makes `parse` return nil, so the caller shows the source instead of a wrong picture.

indirect enum MathNode: Equatable, Sendable {
    case row([MathNode])
    case symbol(String, italic: Bool)
    case text(String)
    case script(base: MathNode, sup: MathNode?, sub: MathNode?)
    case fraction(MathNode, MathNode)
    case sqrt(MathNode, index: MathNode?)
    case bigOp(String, sub: MathNode?, sup: MathNode?)
    case delimited(left: String, right: String, MathNode)
    case matrix(rows: [[MathNode]], left: String, right: String, alignment: MatrixAlignment)
    case accent(MathAccent, MathNode)
    case styled(MathStyle, MathNode)
}

enum MathAccent: Equatable, Sendable { case hat, bar, vec, tilde, dot, ddot, overline, underline }
enum MathStyle: Equatable, Sendable { case bold, roman }
enum MatrixAlignment: Equatable, Sendable { case center, aligned }

struct MathParseError: Error {}

enum MathParser {
    static func parse(_ latex: String) -> MathNode? {
        var scanner = MathScanner(Array(latex))
        guard let node = try? scanner.parseRow(), scanner.atEnd else { return nil }
        if case .row(let items) = node, items.isEmpty { return nil }
        return node
    }

    // MARK: tables

    static let greek: [String: (glyph: String, italic: Bool)] = [
        "alpha": ("α", true), "beta": ("β", true), "gamma": ("γ", true), "delta": ("δ", true), "epsilon": ("ϵ", true),
        "varepsilon": ("ε", true), "zeta": ("ζ", true), "eta": ("η", true), "theta": ("θ", true), "vartheta": ("ϑ", true),
        "iota": ("ι", true), "kappa": ("κ", true), "lambda": ("λ", true), "mu": ("μ", true), "nu": ("ν", true),
        "xi": ("ξ", true), "omicron": ("ο", true), "pi": ("π", true), "varpi": ("ϖ", true), "rho": ("ρ", true),
        "varrho": ("ϱ", true), "sigma": ("σ", true), "varsigma": ("ς", true), "tau": ("τ", true), "upsilon": ("υ", true),
        "phi": ("ϕ", true), "varphi": ("φ", true), "chi": ("χ", true), "psi": ("ψ", true), "omega": ("ω", true),
        "Gamma": ("Γ", false), "Delta": ("Δ", false), "Theta": ("Θ", false), "Lambda": ("Λ", false), "Xi": ("Ξ", false),
        "Pi": ("Π", false), "Sigma": ("Σ", false), "Upsilon": ("Υ", false), "Phi": ("Φ", false), "Psi": ("Ψ", false),
        "Omega": ("Ω", false),
    ]

    static let symbols: [String: String] = [
        "pm": "±", "mp": "∓", "times": "×", "cdot": "⋅", "div": "÷", "leq": "≤", "le": "≤", "geq": "≥", "ge": "≥",
        "neq": "≠", "ne": "≠", "approx": "≈", "equiv": "≡", "sim": "∼", "simeq": "≃", "cong": "≅", "propto": "∝",
        "infty": "∞", "partial": "∂", "nabla": "∇", "to": "→", "rightarrow": "→", "leftarrow": "←", "gets": "←",
        "leftrightarrow": "↔", "Rightarrow": "⇒", "Leftarrow": "⇐", "Leftrightarrow": "⇔", "implies": "⇒", "iff": "⇔",
        "mapsto": "↦", "uparrow": "↑", "downarrow": "↓", "in": "∈", "notin": "∉", "ni": "∋", "subset": "⊂",
        "subseteq": "⊆", "supset": "⊃", "supseteq": "⊇", "cup": "∪", "cap": "∩", "emptyset": "∅", "varnothing": "∅",
        "forall": "∀", "exists": "∃", "neg": "¬", "lnot": "¬", "land": "∧", "wedge": "∧", "lor": "∨", "vee": "∨",
        "ldots": "…", "dots": "…", "cdots": "⋯", "vdots": "⋮", "ddots": "⋱", "circ": "∘", "bullet": "∙", "star": "⋆",
        "angle": "∠", "perp": "⊥", "parallel": "∥", "degree": "°", "prime": "′", "ell": "ℓ", "hbar": "ℏ", "Re": "ℜ",
        "Im": "ℑ", "aleph": "ℵ", "oplus": "⊕", "otimes": "⊗", "ll": "≪", "gg": "≫", "mid": "∣", "setminus": "∖",
        "langle": "⟨", "rangle": "⟩", "lfloor": "⌊", "rfloor": "⌋", "lceil": "⌈", "rceil": "⌉",
        "%": "%", "&": "&", "_": "_", "#": "#", "$": "$", "{": "{", "}": "}", "|": "‖", "vert": "|", "Vert": "‖",
    ]

    /// Symbols that read better with a space on each side.
    static let spacedOperators: Set<String> = [
        "+", "−", "=", "<", ">", "≤", "≥", "≠", "≈", "≡", "∼", "≃", "≅", "∝", "±", "∓", "×", "⋅", "÷", "→", "←", "↔",
        "⇒", "⇐", "⇔", "↦", "∈", "∉", "∋", "⊂", "⊆", "⊃", "⊇", "∪", "∩", "∧", "∨", "∘", "∣", "∥", "≪", "≫", "⊕", "⊗", "∖",
    ]

    static let spaces: [String: String] = [",": "\u{2009}", ";": "\u{2005}", ":": "\u{2005}", " ": " ", "quad": "\u{2003}", "qquad": "\u{2003}\u{2003}"]

    /// Operators that carry limits (and whether they sit below/above the symbol or to its side).
    static let bigOperators: [String: (glyph: String, limitsBelow: Bool)] = [
        "sum": ("∑", true), "prod": ("∏", true), "coprod": ("∐", true), "bigcup": ("⋃", true), "bigcap": ("⋂", true),
        "int": ("∫", false), "iint": ("∬", false), "iiint": ("∭", false), "oint": ("∮", false),
        "lim": ("lim", true), "limsup": ("lim sup", true), "liminf": ("lim inf", true),
        "max": ("max", true), "min": ("min", true), "sup": ("sup", true), "inf": ("inf", true),
    ]

    static let functions: Set<String> = [
        "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh", "coth",
        "log", "ln", "lg", "exp", "det", "dim", "ker", "gcd", "deg", "arg", "Pr", "hom", "mod",
    ]

    static let doubleStruck: [String: String] = ["R": "ℝ", "N": "ℕ", "Z": "ℤ", "Q": "ℚ", "C": "ℂ", "P": "ℙ", "H": "ℍ", "E": "𝔼"]
    static let calligraphic: [String: String] = [
        "L": "ℒ", "H": "ℋ", "F": "ℱ", "E": "ℰ", "B": "ℬ", "I": "ℐ", "M": "ℳ", "R": "ℛ", "N": "𝒩", "O": "𝒪", "P": "𝒫",
        "A": "𝒜", "C": "𝒞", "D": "𝒟", "G": "𝒢", "J": "𝒥", "K": "𝒦", "S": "𝒮", "T": "𝒯", "U": "𝒰", "V": "𝒱",
        "W": "𝒲", "X": "𝒳", "Y": "𝒴", "Z": "𝒵", "Q": "𝒬",
    ]

    static let accents: [String: MathAccent] = [
        "hat": .hat, "widehat": .hat, "bar": .bar, "vec": .vec, "overrightarrow": .vec, "tilde": .tilde, "widetilde": .tilde,
        "dot": .dot, "ddot": .ddot, "overline": .overline, "underline": .underline,
    ]

    static let environments: [String: (left: String, right: String, alignment: MatrixAlignment)] = [
        "matrix": ("", "", .center), "pmatrix": ("(", ")", .center), "bmatrix": ("[", "]", .center),
        "Bmatrix": ("{", "}", .center), "vmatrix": ("|", "|", .center), "Vmatrix": ("‖", "‖", .center),
        "cases": ("{", "", .aligned), "array": ("", "", .center), "aligned": ("", "", .aligned), "align": ("", "", .aligned),
        "align*": ("", "", .aligned), "split": ("", "", .aligned), "gathered": ("", "", .center), "equation": ("", "", .center),
        "equation*": ("", "", .center),
    ]
}

// MARK: - scanner

private struct MathScanner {
    let chars: [Character]
    var pos = 0

    init(_ chars: [Character]) { self.chars = chars }

    var atEnd: Bool { pos >= chars.count }

    mutating func skipSpaces() { while pos < chars.count, chars[pos].isWhitespace { pos += 1 } }

    // `\name` or `\<symbol>` at the cursor, without consuming it.
    func peekCommandName() -> String? {
        guard pos < chars.count, chars[pos] == "\\" else { return nil }
        var p = pos + 1
        guard p < chars.count else { return "" }
        if chars[p].isLetter {
            var name = ""
            while p < chars.count, chars[p].isLetter { name.append(chars[p]); p += 1 }
            return name
        }
        return String(chars[p])
    }

    mutating func readCommandName() -> String {
        guard let name = peekCommandName() else { return "" }
        pos += 1 + max(name.count, 1)
        if name.isEmpty { pos = chars.count }
        return name
    }

    var atLineBreak: Bool { pos + 1 < chars.count && chars[pos] == "\\" && chars[pos + 1] == "\\" }

    // MARK: rows and atoms

    mutating func parseRow() throws -> MathNode {
        var items: [MathNode] = []
        while true {
            skipSpaces()
            if atEnd { break }
            let c = chars[pos]
            if c == "}" || c == "&" || atLineBreak { break }
            if let name = peekCommandName(), name == "right" || name == "end" { break }
            var atom = try parseAtom()
            atom = try parseScripts(for: atom)
            items.append(atom)
        }
        return items.count == 1 ? items[0] : .row(items)
    }

    mutating func parseScripts(for base: MathNode) throws -> MathNode {
        var sup: MathNode?
        var sub: MathNode?
        while true {
            skipSpaces()
            guard !atEnd else { break }
            if chars[pos] == "^" {
                pos += 1
                guard sup == nil else { throw MathParseError() }
                sup = try parseArgument()
            } else if chars[pos] == "_" {
                pos += 1
                guard sub == nil else { throw MathParseError() }
                sub = try parseArgument()
            } else { break }
        }
        if sup == nil, sub == nil { return base }
        if case .bigOp(let glyph, nil, nil) = base { return .bigOp(glyph, sub: sub, sup: sup) }
        return .script(base: base, sup: sup, sub: sub)
    }

    /// `{group}` or one single token (a letter, a digit, a symbol or one command).
    mutating func parseArgument() throws -> MathNode {
        skipSpaces()
        guard pos < chars.count else { throw MathParseError() }
        let c = chars[pos]
        if c == "{" { return try parseGroup() }
        if c == "\\" { return try parseCommand() }
        if c == "}" || c == "^" || c == "_" || c == "&" { throw MathParseError() }
        pos += 1
        if c.isLetter { return .symbol(String(c), italic: true) }
        return .symbol(Self.glyph(for: c), italic: false)
    }

    mutating func parseGroup() throws -> MathNode {
        pos += 1                                           // {
        let inner = try parseRow()
        guard pos < chars.count, chars[pos] == "}" else { throw MathParseError() }
        pos += 1
        return inner
    }

    mutating func parseAtom() throws -> MathNode {
        skipSpaces()
        guard pos < chars.count else { throw MathParseError() }
        let c = chars[pos]
        switch c {
        case "{": return try parseGroup()
        case "\\": return try parseCommand()
        case "^", "_": return .row([])                      // a script with no base: the caller reads the script
        case "}", "&": throw MathParseError()
        default:
            if c.isASCII, c.isNumber {
                var digits = ""
                while pos < chars.count {
                    let d = chars[pos]
                    if d.isASCII, d.isNumber { digits.append(d); pos += 1 }
                    else if d == ".", pos + 1 < chars.count, chars[pos + 1].isASCII, chars[pos + 1].isNumber, !digits.isEmpty { digits.append(d); pos += 1 }
                    else { break }
                }
                return .symbol(digits, italic: false)
            }
            pos += 1
            if c.isLetter { return .symbol(String(c), italic: true) }
            return .symbol(Self.glyph(for: c), italic: false)
        }
    }

    static func glyph(for c: Character) -> String {
        switch c {
        case "-": return "−"
        case "*": return "∗"
        case "'": return "′"
        case "~": return "\u{00A0}"
        default: return String(c)
        }
    }

    // MARK: commands

    mutating func parseCommand() throws -> MathNode {
        let name = readCommandName()
        if let g = MathParser.greek[name] { return .symbol(g.glyph, italic: g.italic) }
        if let space = MathParser.spaces[name] { return .symbol(space, italic: false) }
        if name == "!" { return .row([]) }
        if let op = MathParser.bigOperators[name] { return .bigOp(op.glyph, sub: nil, sup: nil) }
        if MathParser.functions.contains(name) { return .symbol(name, italic: false) }
        if let accent = MathParser.accents[name] { return .accent(accent, try parseArgument()) }
        if let symbol = MathParser.symbols[name] { return .symbol(symbol, italic: false) }

        switch name {
        case "frac", "dfrac", "tfrac":
            let numerator = try parseArgument()
            let denominator = try parseArgument()
            return .fraction(numerator, denominator)
        case "binom", "dbinom", "tbinom":
            let top = try parseArgument()
            let bottom = try parseArgument()
            return .matrix(rows: [[top], [bottom]], left: "(", right: ")", alignment: .center)
        case "sqrt":
            skipSpaces()
            var index: MathNode?
            if pos < chars.count, chars[pos] == "[" {
                guard let close = chars[pos...].firstIndex(of: "]") else { throw MathParseError() }
                var inner = MathScanner(Array(chars[(pos + 1)..<close]))
                index = try inner.parseRow()
                guard inner.atEnd else { throw MathParseError() }
                pos = close + 1
            }
            return .sqrt(try parseArgument(), index: index)
        case "left":
            let left = try parseDelimiter()
            let inner = try parseRow()
            guard peekCommandName() == "right" else { throw MathParseError() }
            _ = readCommandName()
            let right = try parseDelimiter()
            return .delimited(left: left, right: right, inner)
        case "text", "textrm", "mbox", "textit", "textsf":
            return .text(try readBraced())
        case "mathrm", "operatorname", "mathsf", "mathtt":
            return .styled(.roman, try parseArgument())
        case "textbf", "mathbf", "boldsymbol", "bm", "pmb":
            return .styled(.bold, try parseArgument())
        case "mathit":
            return try parseArgument()
        case "mathbb":
            let arg = try parseArgument()
            if case .symbol(let s, _) = arg, let mapped = MathParser.doubleStruck[s] { return .symbol(mapped, italic: false) }
            return arg
        case "mathcal", "mathscr":
            let arg = try parseArgument()
            if case .symbol(let s, _) = arg, let mapped = MathParser.calligraphic[s] { return .symbol(mapped, italic: false) }
            return arg
        case "begin":
            return try parseEnvironment()
        default:
            throw MathParseError()                          // includes a stray \right or \end, and unknown commands
        }
    }

    /// The delimiter after `\left` / `\right`: `.` means none.
    mutating func parseDelimiter() throws -> String {
        skipSpaces()
        guard pos < chars.count else { throw MathParseError() }
        let c = chars[pos]
        if c == "." { pos += 1; return "" }
        if c == "\\" {
            let name = readCommandName()
            if let symbol = MathParser.symbols[name], ["{", "}", "⟨", "⟩", "⌊", "⌋", "⌈", "⌉", "‖", "|"].contains(symbol) { return symbol }
            throw MathParseError()
        }
        guard "()[]|/<>".contains(c) else { throw MathParseError() }
        pos += 1
        return String(c)
    }

    /// Raw text between braces (nested braces allowed), for `\text{…}` and environment names.
    mutating func readBraced() throws -> String {
        skipSpaces()
        guard pos < chars.count, chars[pos] == "{" else { throw MathParseError() }
        var depth = 0
        var out = ""
        while pos < chars.count {
            let c = chars[pos]
            pos += 1
            if c == "{" { depth += 1; if depth == 1 { continue } }
            if c == "}" { depth -= 1; if depth == 0 { return out } }
            out.append(c)
        }
        throw MathParseError()
    }

    mutating func parseEnvironment() throws -> MathNode {
        let environment = try readBraced()
        guard let layout = MathParser.environments[environment] else { throw MathParseError() }
        if environment == "array" {
            skipSpaces()
            if pos < chars.count, chars[pos] == "{" { _ = try readBraced() }   // the column specification
        }
        var rows: [[MathNode]] = [[]]
        while true {
            let cell = try parseRow()
            rows[rows.count - 1].append(cell)
            skipSpaces()
            guard !atEnd else { throw MathParseError() }
            if chars[pos] == "&" { pos += 1; continue }
            if atLineBreak { pos += 2; rows.append([]); continue }
            if peekCommandName() == "end" {
                _ = readCommandName()
                guard try readBraced() == environment else { throw MathParseError() }
                break
            }
            throw MathParseError()
        }
        if let last = rows.last, last.count == 1, case .row(let items) = last[0], items.isEmpty { rows.removeLast() }   // a trailing \\
        guard !rows.isEmpty else { throw MathParseError() }
        return .matrix(rows: rows, left: layout.left, right: layout.right, alignment: layout.alignment)
    }
}

// MARK: - plain text (formulas inside a line of text)

extension MathNode {
    /// A Unicode version: `x²`, `a⁄b`, `√x`, `∑ᵢ₌₁ⁿ i`. Exponents that have no Unicode form fall back to `^(…)`.
    var plainText: String { text(compact: false) }

    fileprivate func text(compact: Bool) -> String {
        switch self {
        case .symbol(let s, _): return s
        case .text(let t): return t
        case .row(let items):
            var out = ""
            for item in items {
                let t = item.text(compact: compact)
                if !compact, case .symbol(let s, _) = item, MathParser.spacedOperators.contains(s) { out += " " + s + " " }
                else if !compact, case .symbol(",", _) = item { out += ", " }
                else if !compact, case .bigOp = item { out += t + " " }
                else { out += t }
            }
            while out.contains("  ") { out = out.replacingOccurrences(of: "  ", with: " ") }
            return out.trimmingCharacters(in: .whitespaces)
        case .script(let base, let sup, let sub):
            return base.text(compact: compact) + Self.scripted(sub, below: true) + Self.scripted(sup, below: false)
        case .fraction(let a, let b):
            return a.wrapped(compact: compact) + "⁄" + b.wrapped(compact: compact)
        case .sqrt(let x, let index):
            let prefix = index.map { Self.scripted($0, below: false, marker: false) } ?? ""
            return prefix + "√" + x.wrapped(compact: compact)
        case .bigOp(let glyph, let sub, let sup):
            return glyph + Self.scripted(sub, below: true) + Self.scripted(sup, below: false)
        case .delimited(let l, let r, let inner):
            return l + inner.text(compact: compact) + r
        case .matrix(let rows, let l, let r, _):
            let body = rows.map { $0.map { $0.text(compact: true) }.joined(separator: " ") }.joined(separator: "; ")
            return l + body + r
        case .accent(let accent, let inner):
            let marks: [MathAccent: String] = [.hat: "\u{0302}", .bar: "\u{0304}", .vec: "\u{20D7}", .tilde: "\u{0303}", .dot: "\u{0307}",
                                               .ddot: "\u{0308}", .overline: "\u{0305}", .underline: "\u{0332}"]
            return inner.text(compact: compact) + (marks[accent] ?? "")
        case .styled(_, let inner):
            return inner.text(compact: compact)
        }
    }

    /// Parentheses around anything that is more than one symbol.
    fileprivate func wrapped(compact: Bool) -> String {
        let t = text(compact: compact)
        if case .symbol = self { return t }
        if case .text = self { return t }
        return t.count <= 1 ? t : "(" + t + ")"
    }

    private static let superscripts: [Character: Character] = {
        var m: [Character: Character] = [:]
        for (a, b) in zip("0123456789+−=()", "⁰¹²³⁴⁵⁶⁷⁸⁹⁺⁻⁼⁽⁾") { m[a] = b }
        for (a, b) in zip("abcdefghijklmnoprstuvwxyz", "ᵃᵇᶜᵈᵉᶠᵍʰⁱʲᵏˡᵐⁿᵒᵖʳˢᵗᵘᵛʷˣʸᶻ") { m[a] = b }
        return m
    }()

    private static let subscripts: [Character: Character] = {
        var m: [Character: Character] = [:]
        for (a, b) in zip("0123456789+−=()", "₀₁₂₃₄₅₆₇₈₉₊₋₌₍₎") { m[a] = b }
        for (a, b) in zip("aehijklmnoprstuvx", "ₐₑₕᵢⱼₖₗₘₙₒₚᵣₛₜᵤᵥₓ") { m[a] = b }
        return m
    }()

    private static func scripted(_ node: MathNode?, below: Bool, marker: Bool = true) -> String {
        guard let node else { return "" }
        let raw = node.text(compact: true)
        let table = below ? subscripts : superscripts
        let mapped = raw.map { table[$0] }
        if !mapped.contains(where: { $0 == nil }) { return String(mapped.compactMap { $0 }) }
        guard marker else { return raw }
        return (below ? "_" : "^") + (raw.count == 1 ? raw : "(" + raw + ")")
    }
}
