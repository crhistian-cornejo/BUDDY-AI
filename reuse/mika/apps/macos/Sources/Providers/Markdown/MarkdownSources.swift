import Foundation

/// An answer from the web is full of links. MIKA shows them as small icons under the answer, so the text itself
/// carries no addresses: a link keeps its label (or disappears when it is only a citation) and a bare address shows
/// only its site. Everything it takes out is returned as a source. Code is never touched.
/// Which links stay in the text, clickable. An answer is untrusted text, so by default no address is clickable (they go to
/// the sources row, which opens http/https only). The exceptions are the user's own workspaces, the places a connector hands
/// files, events and meetings back from: a Google Doc, a Calendar event, a Meet room. Exact hosts, https only, no
/// credentials in the address, no other port. Twin of `linkKept` in apps/windows/src/core/markdown.ts.
enum LinkPolicy {
    static let hosts: Set<String> = [
        "docs.google.com", "drive.google.com", "sheets.google.com", "slides.google.com", "forms.google.com", "meet.google.com",
        "mail.google.com", "calendar.google.com", "contacts.google.com", "github.com", "notion.so", "www.notion.so",
    ]

    static func keeps(_ raw: String) -> Bool {
        guard let parts = URLComponents(string: raw.trimmingCharacters(in: .whitespaces)), parts.scheme?.lowercased() == "https",
              parts.user == nil, parts.password == nil, parts.port == nil, let host = parts.host?.lowercased() else { return false }
        if hosts.contains(host) { return true }
        // A Calendar event's own link lives under www.google.com/calendar/.
        return host == "www.google.com" && parts.path.hasPrefix("/calendar/")
    }
}

/// What a workspace link points to, for the small icon before it.
enum LinkService: String, Sendable { case docs, sheets, slides, forms, drive, calendar, meet, gmail, github, notion }

extension LinkPolicy {
    /// The service a kept link belongs to (a Doc, a Sheet, a Calendar event, a Meet room…), or nil when it has no icon.
    /// Twin of `linkService` in apps/windows/src/core/markdown.ts.
    static func service(for raw: String) -> LinkService? {
        guard keeps(raw), let parts = URLComponents(string: raw.trimmingCharacters(in: .whitespaces)), let host = parts.host?.lowercased() else { return nil }
        let path = parts.path
        switch host {
        case "docs.google.com":
            if path.hasPrefix("/spreadsheets") { return .sheets }
            if path.hasPrefix("/presentation") { return .slides }
            if path.hasPrefix("/forms") { return .forms }
            return .docs
        case "sheets.google.com": return .sheets
        case "slides.google.com": return .slides
        case "forms.google.com": return .forms
        case "drive.google.com": return .drive
        case "calendar.google.com", "www.google.com": return .calendar
        case "meet.google.com": return .meet
        case "mail.google.com": return .gmail
        case "github.com": return .github
        case "notion.so", "www.notion.so": return .notion
        default: return nil
        }
    }
}

enum MarkdownSources {
    static func clean(_ source: String) -> (text: String, sources: [ChatSource]) {
        var sources: [ChatSource] = []
        var out: [String] = []
        var changed = false
        var fence: String?

        func add(_ s: ChatSource) { if !sources.contains(where: { $0.url == s.url }) { sources.append(s) } }

        for line in source.replacingOccurrences(of: "\r\n", with: "\n").components(separatedBy: "\n") {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if let open = fence {
                if trimmed.hasPrefix(open) { fence = nil }
                out.append(line); continue
            }
            if trimmed.hasPrefix("```") { fence = "```"; out.append(line); continue }
            if trimmed.hasPrefix("~~~") { fence = "~~~"; out.append(line); continue }

            if let only = onlyALink(trimmed) { add(only); changed = true; continue }
            let (text, found, didChange) = inline(line)
            found.forEach(add)
            if didChange { changed = true }
            out.append(text)
        }

        // A "Fuentes:" label left at the end has nothing under it any more.
        var trimmedTail = false
        while let last = out.last, last.trimmingCharacters(in: .whitespaces).isEmpty || isSourcesLabel(last) {
            if !last.trimmingCharacters(in: .whitespaces).isEmpty || changed { out.removeLast(); trimmedTail = true } else { break }
        }
        if !changed && !trimmedTail { return (source, []) }
        return (out.joined(separator: "\n"), sources)
    }

    // MARK: lines

    private static let labelPattern = try! NSRegularExpression(
        pattern: #"^(?:#{1,6}\s*)?(?:\*\*|__)?\s*(?:fuentes?|sources?|referencias?|enlaces|links)\s*:?\s*(?:\*\*|__)?\s*:?\s*$"#, options: [.caseInsensitive])

    private static func isSourcesLabel(_ line: String) -> Bool {
        let t = line.trimmingCharacters(in: .whitespaces)
        return labelPattern.firstMatch(in: t, range: NSRange(t.startIndex..., in: t)) != nil
    }

    private static let onlyLinkPattern = try! NSRegularExpression(
        pattern: #"^(?:[-*+•]|\d+[.)])?\s*(?:\[([^\]]*)\]\(((?:[^()\s]|\([^()\s]*\))+)\)|<?(https?://[^\s<>]+)>?)\s*$"#)

    /// A line that is only a link (maybe as a list item): it is all citation, so the line goes away.
    private static func onlyALink(_ line: String) -> ChatSource? {
        let range = NSRange(line.startIndex..., in: line)
        guard let m = onlyLinkPattern.firstMatch(in: line, range: range) else { return nil }
        func group(_ i: Int) -> String? { Range(m.range(at: i), in: line).map { String(line[$0]) } }
        if let url = group(2) { return LinkPolicy.keeps(url) ? nil : ChatSource.make(title: group(1) ?? "", url: url) }
        if let url = group(3) { return LinkPolicy.keeps(stripTrailingPunctuation(url)) ? nil : ChatSource.make(title: "", url: stripTrailingPunctuation(url)) }
        return nil
    }

    // MARK: inline

    private static let genericLabels: Set<String> = [
        "fuente", "fuentes", "source", "sources", "aquí", "aqui", "link", "enlace", "ver más", "ver mas", "leer más",
        "leer mas", "más información", "mas informacion", "here", "read more", "click aquí", "haz clic aquí",
    ]

    private static func stripTrailingPunctuation(_ url: String) -> String {
        var u = url
        while let last = u.last, ".,;:!?\"'".contains(last) { u.removeLast() }
        return u
    }

    private static func inline(_ line: String) -> (text: String, sources: [ChatSource], changed: Bool) {
        let chars = Array(line)
        var out: [Character] = []
        var sources: [ChatSource] = []
        var changed = false
        var i = 0

        func startsWith(_ text: String, at index: Int) -> Bool {
            let t = Array(text)
            return index + t.count <= chars.count && Array(chars[index..<(index + t.count)]) == t
        }

        while i < chars.count {
            let c = chars[i]

            if c == "`", let end = ((i + 1)..<chars.count).first(where: { chars[$0] == "`" }) {     // code span: untouched
                out.append(contentsOf: chars[i...end]); i = end + 1; continue
            }

            if c == "[", out.last != "!", parseLink(chars, at: i) == nil, isOpenLink(chars, at: i) {
                out.append(contentsOf: chars[i...]); break                                          // still streaming: wait
            }

            if c == "[", !(out.last == "!"), let link = parseLink(chars, at: i) {
                // The user's own workspace links stay in the text as links.
                if LinkPolicy.keeps(link.url) { out.append(contentsOf: chars[i..<link.end]); i = link.end; continue }
                changed = true
                if let source = ChatSource.make(title: link.label, url: link.url) {
                    sources.append(source)
                    let label = link.label.trimmingCharacters(in: .whitespaces)
                    let citation = genericLabels.contains(label.lowercased()) || label.isEmpty || label.allSatisfy(\.isNumber)
                    let wrapped = out.last == "(" && link.end < chars.count && chars[link.end] == ")"
                    if wrapped || citation {
                        if wrapped { out.removeLast(); i = link.end + 1 } else { i = link.end }
                        let next: Character? = i < chars.count ? chars[i] : nil
                        if out.last == " ", next == nil || ".,;:!?)".contains(next!) || wrapped { out.removeLast() }
                        continue
                    }
                    out.append(contentsOf: label)
                } else {
                    out.append(contentsOf: link.label)                                             // unsafe scheme: label only
                }
                i = link.end
                continue
            }

            let wordBefore = out.last.map { $0.isLetter || $0.isNumber } ?? false
            let bracketed = c == "<" && (startsWith("<http://", at: i) || startsWith("<https://", at: i))
            if !wordBefore, bracketed || startsWith("http://", at: i) || startsWith("https://", at: i) {
                var j = bracketed ? i + 1 : i
                let from = j
                while j < chars.count, !chars[j].isWhitespace, !"<>)]\"'".contains(chars[j]) { j += 1 }
                let raw = String(chars[from..<j])
                let url = stripTrailingPunctuation(raw)
                if LinkPolicy.keeps(url), let host = URL(string: url)?.host {
                    changed = true
                    out.append(contentsOf: "[\(host)](\(url))")
                    i = from + url.count
                    if bracketed, i < chars.count, chars[i] == ">" { i += 1 }
                    continue
                }
                if let source = ChatSource.make(title: "", url: url), source.host.contains(".") {
                    changed = true
                    sources.append(source)
                    out.append(contentsOf: source.host)
                    i = from + url.count
                    if bracketed, i < chars.count, chars[i] == ">" { i += 1 }
                    continue
                }
            }

            out.append(c)
            i += 1
        }
        return (String(out), sources, changed)
    }

    /// `[label](` with no closing parenthesis yet.
    private static func isOpenLink(_ chars: [Character], at start: Int) -> Bool {
        var i = start + 1
        while i < chars.count, chars[i] != "]" {
            if chars[i] == "[" || chars[i] == "\n" { return false }
            i += 1
        }
        return i + 1 < chars.count && chars[i] == "]" && chars[i + 1] == "("
    }

    /// `[label](address)` starting at `start`. A link that is not complete yet (it is still streaming) is not a link.
    private static func parseLink(_ chars: [Character], at start: Int) -> (label: String, url: String, end: Int)? {
        var i = start + 1
        var label: [Character] = []
        while i < chars.count, chars[i] != "]" {
            if chars[i] == "[" || chars[i] == "\n" { return nil }
            label.append(chars[i]); i += 1
        }
        guard i + 1 < chars.count, chars[i] == "]", chars[i + 1] == "(" else { return nil }
        i += 2
        var url: [Character] = []
        var depth = 0
        while i < chars.count {
            let c = chars[i]
            if c == "(" { depth += 1 }
            else if c == ")" {
                if depth == 0 { return (String(label), String(url).trimmingCharacters(in: .whitespaces), i + 1) }
                depth -= 1
            } else if c == "\n" { return nil }
            url.append(c); i += 1
        }
        return nil
    }
}
