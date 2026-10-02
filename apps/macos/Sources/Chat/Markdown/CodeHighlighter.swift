// Ported from MIKA (MIT, © MIKA contributors, revision d050bc5): apps/macos/Sources/Providers/Markdown/CodeHighlighter.swift
import Foundation

// Syntax colouring for the code blocks of an answer. Pure Foundation, so the tests compile it as is. Twin of
// apps/windows/src/core/highlight.ts: the same languages, the same scanner, the same token kinds, the same test cases.
//
// It is a scanner, not a parser: it knows comments, strings, numbers, a short keyword list per language and little else,
// which is enough to make code readable and cheap enough to run on every delta while an answer streams. The tokens always
// cover the whole input, in order, so joining their text gives the input back, even for broken code. Unknown languages and
// very long blocks come back as one plain token.

enum CodeTokenKind: String, Sendable { case kw, str, com, num, type, fn, prop, tag, add, del, hunk, plain }

struct CodeToken: Equatable, Sendable {
    var kind: CodeTokenKind
    var text: String
}

enum CodeHighlighter {
    /// Longer than this is shown plain: a pasted log should not cost a scan per streamed word.
    static let maxLength = 30_000

    private struct Cfg {
        var kw: Set<String>
        var line: [String] = []
        var block: (String, String)? = nil
        var triple: [String] = []
        var quotes: [Unicode.Scalar] = ["\"", "'"]
        var template = false
        var charQuote = false
        var ci = false
        var keyColon = false
        var dash = false
        var dollar = false
        var at = false
        var hexColor = false
        var hashWord = false
    }

    private static func words(_ s: String) -> Set<String> { Set(s.split(whereSeparator: { $0 == " " || $0 == "\n" }).map(String.init)) }

    private static let jsKW = words("var let const function return if else for while do switch case break continue default new delete typeof instanceof in of this super class extends import export from as async await yield try catch finally throw void null undefined true false static get set interface type enum implements public private protected readonly declare namespace abstract keyof satisfies is")
    private static let swiftKW = words("let var func class struct enum protocol extension import return if else guard switch case default for while repeat break continue in is as try catch throw throws rethrows do defer init deinit self Self super nil true false static public private fileprivate internal open final lazy weak unowned mutating nonmutating override some any async await actor where associatedtype typealias subscript inout operator indirect convenience required")
    private static let pyKW = words("def class return if elif else for while break continue pass import from as try except finally raise with lambda yield global nonlocal assert del in is not and or None True False async await match case self cls")
    private static let rustKW = words("fn let mut const static struct enum impl trait pub use mod crate self Self super match if else for while loop break continue return as in where type unsafe async await move ref dyn box true false extern macro_rules")
    private static let goKW = words("func var const type struct interface map chan package import return if else for range switch case default break continue go defer select fallthrough goto nil true false iota")
    private static let javaKW = words("class interface enum extends implements import package return if else for while do switch case default break continue new this super try catch finally throw throws public private protected static final abstract void int long short byte float double boolean char null true false instanceof synchronized volatile transient native var record sealed permits fun val when object companion data override open internal lateinit by is in out sealed suspend")
    private static let cKW = words("auto break case char const continue default do double else enum extern float for goto if inline int long register return short signed sizeof static struct switch typedef union unsigned void volatile while class namespace template typename public private protected virtual new delete this nullptr true false bool using try catch throw operator constexpr override final noexcept include define ifdef ifndef endif pragma string var foreach in is as null abstract sealed readonly lock async await")
    private static let shKW = words("if then else elif fi for while until do done case esac function in select time return exit break continue export local readonly declare unset source alias echo cd set shift trap eval exec")
    private static let sqlKW = words("select from where and or not null is in like between join inner outer left right full cross on as group by order having limit offset insert into values update set delete create table alter drop index view primary key foreign references unique default check constraint distinct union all exists case when then else end asc desc count sum avg min max with returning begin commit rollback true false cascade if truncate")
    private static let phpKW = words("function class public private protected static return if else elseif foreach for while do switch case break continue new echo print null true false array use namespace extends implements interface trait try catch finally throw const abstract final isset unset empty require include require_once include_once as fn match")

    private static func cFamily(_ kw: Set<String>) -> Cfg { Cfg(kw: kw, line: ["//"], block: ("/*", "*/")) }

    private static let configs: [String: Cfg] = {
        var js = cFamily(jsKW); js.quotes = ["\"", "'", "`"]; js.template = true; js.at = true
        var swift = cFamily(swiftKW); swift.at = true
        var python = Cfg(kw: pyKW, line: ["#"], triple: ["\"\"\"", "'''"]); python.at = true
        var rust = cFamily(rustKW); rust.charQuote = true
        var go = cFamily(goKW); go.quotes = ["\"", "'", "`"]; go.template = true
        var java = cFamily(javaKW); java.at = true
        let c = cFamily(cKW)
        let shell = Cfg(kw: shKW, line: ["#"], quotes: ["\"", "'", "`"], dollar: true, hashWord: true)
        let sql = Cfg(kw: sqlKW, line: ["--"], block: ("/*", "*/"), quotes: ["'", "\""], ci: true)
        let yaml = Cfg(kw: words("true false null yes no on off"), line: ["#"], keyColon: true, dash: true, hashWord: true)
        let json = Cfg(kw: words("true false null"), quotes: ["\""], keyColon: true)
        var css = Cfg(kw: words("important inherit initial unset auto none"), block: ("/*", "*/"), keyColon: true, dash: true)
        css.at = true; css.hexColor = true
        var php = cFamily(phpKW); php.line = ["//", "#"]; php.dollar = true
        return ["js": js, "swift": swift, "python": python, "rust": rust, "go": go, "java": java, "c": c, "shell": shell, "sql": sql,
                "yaml": yaml, "json": json, "css": css, "php": php]
    }()

    private static let aliases: [String: String] = [
        "js": "js", "jsx": "js", "javascript": "js", "mjs": "js", "cjs": "js", "ts": "js", "tsx": "js", "typescript": "js", "node": "js",
        "swift": "swift", "py": "python", "python": "python", "python3": "python", "rust": "rust", "rs": "rust", "go": "go", "golang": "go",
        "java": "java", "kotlin": "java", "kt": "java", "scala": "java", "dart": "java", "c": "c", "h": "c", "cpp": "c", "c++": "c",
        "cc": "c", "hpp": "c", "csharp": "c", "cs": "c", "c#": "c", "objc": "c", "bash": "shell", "sh": "shell", "zsh": "shell",
        "shell": "shell", "console": "shell", "fish": "shell", "sql": "sql", "postgres": "sql", "postgresql": "sql", "mysql": "sql",
        "sqlite": "sql", "yaml": "yaml", "yml": "yaml", "toml": "yaml", "json": "json", "jsonc": "json", "json5": "json", "css": "css",
        "scss": "css", "less": "css", "html": "html", "xml": "html", "svg": "html", "xhtml": "html", "vue": "html", "plist": "html",
        "php": "php", "diff": "diff", "patch": "diff",
    ]

    /// The language family a fence's info string names (`ts`, `TypeScript`, `bash title="x"`), or nil when it is unknown.
    static func family(of info: String?) -> String? {
        let first = (info ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            .split(whereSeparator: { $0 == " " || $0 == "\t" || $0 == "{" || $0 == "," }).first.map(String.init)?.lowercased() ?? ""
        let name = first.hasPrefix("language-") ? String(first.dropFirst(9)) : first
        return aliases[name]
    }

    /// `code` as coloured spans. `language` is the fence's info string.
    static func highlight(_ code: String, language: String?) -> [CodeToken] {
        guard let family = family(of: language), code.utf16.count <= maxLength else {
            return code.isEmpty ? [] : [CodeToken(kind: .plain, text: code)]
        }
        let s = Array(code.unicodeScalars)
        switch family {
        case "html": return scanHTML(s)
        case "diff": return scanDiff(code)
        default: return scanGeneric(s, configs[family]!)
        }
    }

    // MARK: scanning

    private static func isIdentStart(_ c: Unicode.Scalar) -> Bool {
        (c >= "a" && c <= "z") || (c >= "A" && c <= "Z") || c == "_" || c == "$" || c.value > 127
    }
    private static func isIdentPart(_ c: Unicode.Scalar) -> Bool { isIdentStart(c) || isDigit(c) }
    private static func isDigit(_ c: Unicode.Scalar) -> Bool { c >= "0" && c <= "9" }
    private static func isHex(_ c: Unicode.Scalar) -> Bool { isDigit(c) || (c >= "a" && c <= "f") || (c >= "A" && c <= "F") }
    private static func isLetter(_ c: Unicode.Scalar) -> Bool { (c >= "a" && c <= "z") || (c >= "A" && c <= "Z") }
    private static func isSpace(_ c: Unicode.Scalar) -> Bool { c == " " || c == "\t" || c == "\n" || c == "\r" }

    private static func at(_ s: [Unicode.Scalar], _ i: Int) -> Unicode.Scalar? { i >= 0 && i < s.count ? s[i] : nil }

    private static func starts(_ s: [Unicode.Scalar], _ p: String, at i: Int) -> Bool {
        let scalars = Array(p.unicodeScalars)
        guard i + scalars.count <= s.count else { return false }
        for (k, c) in scalars.enumerated() where s[i + k] != c { return false }
        return true
    }

    private static func find(_ s: [Unicode.Scalar], _ p: String, from i: Int) -> Int? {
        let n = p.unicodeScalars.count
        var j = i
        while j + n <= s.count { if starts(s, p, at: j) { return j }; j += 1 }
        return nil
    }

    private static func text(_ s: [Unicode.Scalar], _ a: Int, _ b: Int) -> String {
        var view = String.UnicodeScalarView()
        view.append(contentsOf: s[a..<b])
        return String(view)
    }

    private static func push(_ out: inout [CodeToken], _ kind: CodeTokenKind, _ text: String) {
        guard !text.isEmpty else { return }
        if let last = out.last, last.kind == kind { out[out.count - 1].text += text } else { out.append(CodeToken(kind: kind, text: text)) }
    }

    private static func nextNonSpace(_ s: [Unicode.Scalar], _ from: Int) -> Unicode.Scalar? {
        var j = from
        while j < s.count, s[j] == " " || s[j] == "\t" { j += 1 }
        return at(s, j)
    }

    /// The end of the number that starts at `i`.
    private static func numberEnd(_ s: [Unicode.Scalar], _ i: Int) -> Int {
        var j = i
        if s[j] == "0", let p = at(s, j + 1), "xXbBoO".unicodeScalars.contains(p) {
            j += 2
            while j < s.count, isHex(s[j]) || s[j] == "_" { j += 1 }
            return j
        }
        while j < s.count, isDigit(s[j]) || s[j] == "_" { j += 1 }
        if at(s, j) == ".", let d = at(s, j + 1), isDigit(d) {
            j += 1
            while j < s.count, isDigit(s[j]) || s[j] == "_" { j += 1 }
        }
        if let e = at(s, j), e == "e" || e == "E" {
            if let d = at(s, j + 1), isDigit(d) { j += 2; while j < s.count, isDigit(s[j]) { j += 1 } }
            else if let sign = at(s, j + 1), sign == "+" || sign == "-", let d = at(s, j + 2), isDigit(d) {
                j += 3; while j < s.count, isDigit(s[j]) { j += 1 }
            }
        }
        while j < s.count, isLetter(s[j]) || s[j] == "%" { j += 1 }
        return j
    }

    private static func scanGeneric(_ s: [Unicode.Scalar], _ cfg: Cfg) -> [CodeToken] {
        var out: [CodeToken] = []
        var i = 0
        while i < s.count {
            let c = s[i]
            // comments
            if let prefix = cfg.line.first(where: { p in
                starts(s, p, at: i) && (!cfg.hashWord || p != "#" || i == 0 || isSpace(s[i - 1]))
            }) {
                _ = prefix
                var j = i
                while j < s.count, s[j] != "\n" { j += 1 }
                push(&out, .com, text(s, i, j)); i = j; continue
            }
            if let block = cfg.block, starts(s, block.0, at: i) {
                let end = find(s, block.1, from: i + block.0.unicodeScalars.count)
                let j = end.map { $0 + block.1.unicodeScalars.count } ?? s.count
                push(&out, .com, text(s, i, j)); i = j; continue
            }
            // strings
            if let triple = cfg.triple.first(where: { starts(s, $0, at: i) }) {
                let end = find(s, triple, from: i + 3)
                let j = end.map { $0 + 3 } ?? s.count
                push(&out, .str, text(s, i, j)); i = j; continue
            }
            if cfg.quotes.contains(c) {
                if c == "'", cfg.charQuote, !isCharLiteral(s, i) { push(&out, .plain, "'"); i += 1; continue }
                let multiline = c == "`" && cfg.template
                var j = i + 1
                while j < s.count, s[j] != c {
                    if s[j] == "\\", j + 1 < s.count { j += 1 }
                    else if s[j] == "\n", !multiline { break }
                    j += 1
                }
                if at(s, j) == c { j += 1 }
                let key = cfg.keyColon && nextNonSpace(s, j) == ":"
                push(&out, key ? .prop : .str, text(s, i, j)); i = j; continue
            }
            // numbers and colours
            if isDigit(c) || (c == "." && at(s, i + 1).map(isDigit) == true && !(at(s, i - 1).map(isIdentPart) ?? false)) {
                let j = numberEnd(s, i)
                push(&out, .num, text(s, i, j)); i = j; continue
            }
            if cfg.hexColor, c == "#", at(s, i + 1).map(isHex) == true {
                var j = i + 1
                while j < s.count, isHex(s[j]) { j += 1 }
                push(&out, .num, text(s, i, j)); i = j; continue
            }
            // variables, attributes
            if cfg.dollar, c == "$", let n = at(s, i + 1), isIdentStart(n) || n == "{" {
                var j = i + 1
                if s[j] == "{" { j = find(s, "}", from: j).map { $0 + 1 } ?? s.count }
                else { while j < s.count, isIdentPart(s[j]) { j += 1 } }
                push(&out, .prop, text(s, i, j)); i = j; continue
            }
            if cfg.at, c == "@", let n = at(s, i + 1), isIdentStart(n) {
                var j = i + 1
                while j < s.count, isIdentPart(s[j]) || (cfg.dash && s[j] == "-") { j += 1 }
                push(&out, .kw, text(s, i, j)); i = j; continue
            }
            // words
            if isIdentStart(c) {
                var j = i + 1
                while j < s.count, isIdentPart(s[j]) || (cfg.dash && s[j] == "-" && at(s, j + 1).map(isIdentPart) == true) { j += 1 }
                let word = text(s, i, j)
                let lookup = cfg.ci ? word.lowercased() : word
                var kind = CodeTokenKind.plain
                if cfg.kw.contains(lookup) { kind = .kw }
                else if cfg.keyColon, nextNonSpace(s, j) == ":", at(s, j + 1) != ":" { kind = .prop }
                else if at(s, j) == "(" { kind = .fn }
                else if let first = word.unicodeScalars.first, first >= "A" && first <= "Z",
                        word.unicodeScalars.count == 1 || word.unicodeScalars.contains(where: { $0 >= "a" && $0 <= "z" }) { kind = .type }
                push(&out, kind, word); i = j; continue
            }
            push(&out, .plain, text(s, i, i + 1)); i += 1
        }
        return out
    }

    /// `'x'` or `'\n'` (a Rust character) rather than a lifetime.
    private static func isCharLiteral(_ s: [Unicode.Scalar], _ i: Int) -> Bool {
        guard let a = at(s, i + 1), a != "'", a != "\n" else { return false }
        if a == "\\" { return at(s, i + 2) != nil && at(s, i + 2) != "\n" && at(s, i + 3) == "'" }
        return at(s, i + 2) == "'"
    }

    private static func scanHTML(_ s: [Unicode.Scalar]) -> [CodeToken] {
        var out: [CodeToken] = []
        var i = 0
        func tagChar(_ c: Unicode.Scalar) -> Bool { "/!?:_.-".unicodeScalars.contains(c) || isLetter(c) || isDigit(c) }
        while i < s.count {
            if starts(s, "<!--", at: i) {
                let end = find(s, "-->", from: i + 4)
                let j = end.map { $0 + 3 } ?? s.count
                push(&out, .com, text(s, i, j)); i = j; continue
            }
            if s[i] == "<", let n = at(s, i + 1), isLetter(n) || n == "/" || n == "!" || n == "?" {
                var j = i + 1
                while j < s.count, tagChar(s[j]) { j += 1 }
                push(&out, .tag, text(s, i, j)); i = j
                while i < s.count, s[i] != ">" {
                    let c = s[i]
                    if c == "\"" || c == "'" {
                        var k = i + 1
                        while k < s.count, s[k] != c { k += 1 }
                        if at(s, k) == c { k += 1 }
                        push(&out, .str, text(s, i, k)); i = k
                    } else if isLetter(c) || c == "_" || c == ":" || c == "@" {
                        var k = i + 1
                        while k < s.count, isLetter(s[k]) || isDigit(s[k]) || "_:.@-".unicodeScalars.contains(s[k]) { k += 1 }
                        push(&out, .prop, text(s, i, k)); i = k
                    } else if c == "/", at(s, i + 1) == ">" { break }
                    else { push(&out, .plain, text(s, i, i + 1)); i += 1 }
                }
                if at(s, i) == "/", at(s, i + 1) == ">" { push(&out, .tag, "/>"); i += 2 }
                else if at(s, i) == ">" { push(&out, .tag, ">"); i += 1 }
                continue
            }
            push(&out, .plain, text(s, i, i + 1)); i += 1
        }
        return out
    }

    private static func scanDiff(_ code: String) -> [CodeToken] {
        var out: [CodeToken] = []
        var line = ""
        func flush() {
            guard !line.isEmpty else { return }
            let kind: CodeTokenKind = line.hasPrefix("+++") || line.hasPrefix("---") ? .hunk
                : line.hasPrefix("+") ? .add : line.hasPrefix("-") ? .del : line.hasPrefix("@@") ? .hunk : .plain
            push(&out, kind, line)
            line = ""
        }
        for scalar in code.unicodeScalars {
            line.unicodeScalars.append(scalar)
            if scalar == "\n" { flush() }
        }
        flush()
        return out
    }
}
