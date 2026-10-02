// Ported from MIKA (MIT, © MIKA contributors, revision d050bc5): apps/macos/Sources/Providers/Markdown/MarkdownParser.swift
import Foundation

// A small, forgiving markdown reader for chat answers. It only builds a tree of blocks: the text of a paragraph or a
// list item stays as source and is styled later (bold, italic, code, links and inline maths). Everything is text:
// no HTML is ever interpreted. It must cope with half-written answers, because it runs while the text is streaming.

struct MarkdownListItem: Equatable, Sendable {
    var text: String
    var children: [MarkdownBlock] = []
}

enum TableAlignment: Equatable, Sendable { case leading, center, trailing }

indirect enum MarkdownBlock: Equatable, Sendable {
    case heading(level: Int, text: String)
    case paragraph(String)
    case bulletList([MarkdownListItem])
    case orderedList(start: Int, items: [MarkdownListItem])
    case codeBlock(language: String?, code: String)
    case quote([MarkdownBlock])
    case rule
    case table(header: [String], alignments: [TableAlignment], rows: [[String]])
    case math(String)
}

enum MarkdownParser {
    static func parse(_ source: String) -> [MarkdownBlock] {
        blocks(from: source.replacingOccurrences(of: "\r\n", with: "\n").components(separatedBy: "\n"))
    }

    // MARK: - blocks

    private static func blocks(from lines: [String]) -> [MarkdownBlock] {
        var result: [MarkdownBlock] = []
        var paragraph: [String] = []
        var i = 0

        func flushParagraph() {
            if !paragraph.isEmpty {
                result.append(.paragraph(paragraph.joined(separator: "\n")))
                paragraph = []
            }
        }

        while i < lines.count {
            let line = lines[i]
            let trimmed = line.trimmingCharacters(in: .whitespaces)

            if trimmed.isEmpty { flushParagraph(); i += 1; continue }

            if let marker = fenceMarker(trimmed) {
                flushParagraph()
                let info = String(trimmed.dropFirst(3)).trimmingCharacters(in: .whitespaces)
                var code: [String] = []
                i += 1
                while i < lines.count, !lines[i].trimmingCharacters(in: .whitespaces).hasPrefix(marker) {
                    code.append(lines[i])
                    i += 1
                }
                i += 1                                       // the closing fence (or the end of the text)
                let language = info.split(separator: " ").first.map(String.init)
                result.append(.codeBlock(language: language, code: code.joined(separator: "\n")))
                continue
            }

            if let (formula, next) = displayMath(lines, at: i) {
                flushParagraph(); result.append(.math(formula)); i = next; continue
            }
            if let heading = heading(trimmed) { flushParagraph(); result.append(heading); i += 1; continue }
            if isRule(trimmed) { flushParagraph(); result.append(.rule); i += 1; continue }

            if trimmed.hasPrefix(">") {
                flushParagraph()
                var quoted: [String] = []
                while i < lines.count, lines[i].trimmingCharacters(in: .whitespaces).hasPrefix(">") {
                    var content = String(lines[i].trimmingCharacters(in: .whitespaces).dropFirst())
                    if content.hasPrefix(" ") { content.removeFirst() }
                    quoted.append(content)
                    i += 1
                }
                result.append(.quote(blocks(from: quoted)))
                continue
            }

            if let (list, next) = list(lines, at: i) { flushParagraph(); result.append(list); i = next; continue }
            if let (table, next) = table(lines, at: i) { flushParagraph(); result.append(table); i = next; continue }

            paragraph.append(trimmed)
            i += 1
        }
        flushParagraph()
        return result
    }

    private static func fenceMarker(_ trimmed: String) -> String? {
        if trimmed.hasPrefix("```") { return "```" }
        if trimmed.hasPrefix("~~~") { return "~~~" }
        return nil
    }

    /// `$$ … $$` or `\[ … \]`, on one line or across several. Never closed → nil (it stays text until it is).
    private static func displayMath(_ lines: [String], at start: Int) -> (String, Int)? {
        let trimmed = lines[start].trimmingCharacters(in: .whitespaces)
        for (open, close) in [("$$", "$$"), ("\\[", "\\]")] where trimmed.hasPrefix(open) {
            let rest = String(trimmed.dropFirst(open.count))
            if let end = rest.range(of: close) {
                let after = rest[end.upperBound...].trimmingCharacters(in: .whitespaces)
                return after.isEmpty ? (String(rest[..<end.lowerBound]).trimmingCharacters(in: .whitespacesAndNewlines), start + 1) : nil
            }
            var content = rest.trimmingCharacters(in: .whitespaces).isEmpty ? [] : [rest]
            var j = start + 1
            while j < lines.count {
                if let end = lines[j].range(of: close) {
                    content.append(String(lines[j][..<end.lowerBound]))
                    return (content.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines), j + 1)
                }
                content.append(lines[j])
                j += 1
            }
            return nil
        }
        return nil
    }

    private static func heading(_ trimmed: String) -> MarkdownBlock? {
        let hashes = trimmed.prefix { $0 == "#" }.count
        guard (1...6).contains(hashes), trimmed.dropFirst(hashes).first == " " else { return nil }
        var text = trimmed.dropFirst(hashes).trimmingCharacters(in: .whitespaces)
        while text.hasSuffix("#") { text.removeLast() }
        return .heading(level: hashes, text: text.trimmingCharacters(in: .whitespaces))
    }

    private static func isRule(_ trimmed: String) -> Bool {
        let chars = trimmed.filter { $0 != " " }
        guard chars.count >= 3, let first = chars.first, "-*_".contains(first) else { return false }
        return chars.allSatisfy { $0 == first }
    }

    // MARK: - lists

    private struct ListMarker { var indent: Int; var ordered: Bool; var number: Int; var text: String }

    private static func indent(of line: String) -> Int { line.prefix { $0 == " " || $0 == "\t" }.count }

    private static func listMarker(_ line: String) -> ListMarker? {
        let indent = indent(of: line)
        let rest = line.dropFirst(indent)
        if let first = rest.first, "-*+".contains(first), rest.dropFirst().first == " " {
            return ListMarker(indent: indent, ordered: false, number: 0, text: String(rest.dropFirst(2)))
        }
        let digits = rest.prefix { $0.isASCII && $0.isNumber }
        if !digits.isEmpty, digits.count <= 9, let number = Int(digits) {
            let after = rest.dropFirst(digits.count)
            if let mark = after.first, mark == "." || mark == ")", after.dropFirst().first == " " {
                return ListMarker(indent: indent, ordered: true, number: number, text: String(after.dropFirst(2)))
            }
        }
        return nil
    }

    private static func list(_ lines: [String], at start: Int) -> (MarkdownBlock, Int)? {
        guard let first = listMarker(lines[start]) else { return nil }
        var items: [MarkdownListItem] = []
        var i = start

        while i < lines.count {
            guard let marker = listMarker(lines[i]), marker.indent == first.indent, marker.ordered == first.ordered else { break }
            var item = MarkdownListItem(text: marker.text)
            i += 1
            var inner: [String] = []
            while i < lines.count {
                let current = lines[i]
                if current.trimmingCharacters(in: .whitespaces).isEmpty {
                    var j = i + 1
                    while j < lines.count, lines[j].trimmingCharacters(in: .whitespaces).isEmpty { j += 1 }
                    if j < lines.count, indent(of: lines[j]) > first.indent { inner.append(""); i += 1; continue }
                    break
                }
                if indent(of: current) > first.indent {
                    inner.append(String(current.dropFirst(min(indent(of: current), first.indent + 2))))
                    i += 1
                    continue
                }
                break
            }
            if !inner.isEmpty { item.children = blocks(from: inner) }
            items.append(item)

            // a blank line between two items of the same list does not end it
            var j = i
            while j < lines.count, lines[j].trimmingCharacters(in: .whitespaces).isEmpty { j += 1 }
            if j > i, j < lines.count, let next = listMarker(lines[j]), next.indent == first.indent, next.ordered == first.ordered {
                i = j
            }
        }
        return (first.ordered ? .orderedList(start: first.number, items: items) : .bulletList(items), i)
    }

    // MARK: - tables

    private static func cells(of row: String) -> [String] {
        var trimmed = row.trimmingCharacters(in: .whitespaces)
        if trimmed.hasPrefix("|") { trimmed.removeFirst() }
        if trimmed.hasSuffix("|") { trimmed.removeLast() }
        return trimmed.components(separatedBy: "|").map { $0.trimmingCharacters(in: .whitespaces) }
    }

    private static func alignments(of separator: String) -> [TableAlignment]? {
        guard separator.contains("-") else { return nil }
        let parts = cells(of: separator)
        var result: [TableAlignment] = []
        for part in parts {
            let core = part.trimmingCharacters(in: CharacterSet(charactersIn: ":"))
            guard !core.isEmpty, core.allSatisfy({ $0 == "-" }) else { return nil }
            let left = part.hasPrefix(":"), right = part.hasSuffix(":")
            result.append(left && right ? .center : right ? .trailing : .leading)
        }
        return result.isEmpty ? nil : result
    }

    private static func table(_ lines: [String], at start: Int) -> (MarkdownBlock, Int)? {
        guard start + 1 < lines.count, lines[start].contains("|"), let aligns = alignments(of: lines[start + 1]) else { return nil }
        let header = cells(of: lines[start])
        var rows: [[String]] = []
        var i = start + 2
        while i < lines.count, lines[i].contains("|"), !lines[i].trimmingCharacters(in: .whitespaces).isEmpty {
            rows.append(cells(of: lines[i]))
            i += 1
        }
        return (.table(header: header, alignments: aligns, rows: rows), i)
    }
}

// MARK: - inline maths

enum InlineSegment: Equatable, Sendable {
    case text(String)
    case math(String)
}

enum MarkdownTask {
    /// A task-list item: `[ ] algo` / `[x] algo`.
    static func marker(_ text: String) -> (checked: Bool, rest: String)? {
        let chars = Array(text)
        guard chars.count >= 4, chars[0] == "[", chars[2] == "]", chars[1] == " " || chars[1] == "x" || chars[1] == "X",
              chars[3] == " " || chars[3] == "\t" else { return nil }
        let rest = String(chars[3...]).drop(while: { $0 == " " || $0 == "\t" })
        return (chars[1] != " ", String(rest))
    }
}

enum MarkdownInline {
    /// Splits a line of text into plain markdown text and inline formulas (`$x$`, `$$x$$`, `\(x\)`).
    /// Money ("$5 and $10"), code spans, escaped dollars and unclosed or multi-line dollars are left as text.
    static func segments(_ source: String) -> [InlineSegment] {
        let chars = Array(source)
        var out: [InlineSegment] = []
        var buffer = ""
        var i = 0

        func flush() { if !buffer.isEmpty { out.append(.text(buffer)); buffer = "" } }

        while i < chars.count {
            let c = chars[i]
            if c == "\\", i + 1 < chars.count {
                if chars[i + 1] == "(", let end = find(["\\", ")"], in: chars, from: i + 2) {
                    flush(); out.append(.math(String(chars[(i + 2)..<end]).trimmingCharacters(in: .whitespaces))); i = end + 2; continue
                }
                buffer.append(c); buffer.append(chars[i + 1]); i += 2          // keep escapes for the markdown pass
                continue
            }
            if c == "`", let end = ((i + 1)..<chars.count).first(where: { chars[$0] == "`" }) {
                buffer.append(String(chars[i...end])); i = end + 1; continue   // code span: untouched
            }
            if c == "$" {
                if i + 1 < chars.count, chars[i + 1] == "$", let end = find(["$", "$"], in: chars, from: i + 2), end > i + 2 {
                    flush(); out.append(.math(String(chars[(i + 2)..<end]).trimmingCharacters(in: .whitespaces))); i = end + 2; continue
                }
                if let end = closingDollar(chars, from: i + 1) {
                    flush(); out.append(.math(String(chars[(i + 1)..<end]))); i = end + 1; continue
                }
            }
            buffer.append(c)
            i += 1
        }
        flush()
        return out
    }

    private static func find(_ pattern: [Character], in chars: [Character], from start: Int) -> Int? {
        guard start <= chars.count - pattern.count else { return nil }
        for i in start...(chars.count - pattern.count) where Array(chars[i..<(i + pattern.count)]) == pattern { return i }
        return nil
    }

    /// A closing `$` needs a formula that does not start or end with a space, stays on one line and is not
    /// followed by a digit ("$5 and $10" is money).
    private static func closingDollar(_ chars: [Character], from start: Int) -> Int? {
        guard start < chars.count, !chars[start].isWhitespace, chars[start] != "$" else { return nil }
        var j = start
        while j < chars.count {
            let c = chars[j]
            if c == "\n" { return nil }
            if c == "\\" { j += 2; continue }
            if c == "$", !chars[j - 1].isWhitespace, j + 1 >= chars.count || !chars[j + 1].isNumber { return j }
            j += 1
        }
        return nil
    }
}
