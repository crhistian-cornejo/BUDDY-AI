import Foundation

/// Turns the NDJSON of `claude -p --output-format stream-json --include-partial-messages` into provider events.
/// Thinking deltas and tool arguments are never exposed.
struct ClaudeStreamParser {
    private var sawDelta = false
    /// Tool calls seen so far, to know which page a WebFetch result belongs to.
    private var fetchedURLs: [String: String] = [:]

    mutating func feed(_ line: String) -> [ProviderEvent] {
        guard let data = line.data(using: .utf8),
              let obj = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              let type = obj["type"] as? String else { return [] }

        switch type {
        case "system":
            switch obj["subtype"] as? String {
            case "init":
                sawDelta = false
                if let id = obj["session_id"] as? String { return [.session(id)] }
                return []
            case "api_retry": return [.progress("Reintentando…")]
            default: return []
            }

        case "stream_event":
            guard let event = obj["event"] as? [String: Any], let eventType = event["type"] as? String else { return [] }
            if eventType == "content_block_delta",
               let delta = event["delta"] as? [String: Any], delta["type"] as? String == "text_delta",
               let text = delta["text"] as? String, !text.isEmpty {
                sawDelta = true
                return [.delta(text)]
            }
            if eventType == "content_block_start",
               let block = event["content_block"] as? [String: Any], block["type"] as? String == "tool_use",
               let name = block["name"] as? String {
                return [.tool(name: name, summary: "")]
            }
            return []

        case "assistant":
            guard let message = obj["message"] as? [String: Any], let blocks = message["content"] as? [[String: Any]] else { return [] }
            var events: [ProviderEvent] = []
            if !sawDelta {
                let text = blocks.filter { $0["type"] as? String == "text" }
                    .compactMap { $0["text"] as? String }.joined(separator: "\n")
                if !text.isEmpty { events.append(.delta(text)) }
            }
            // The complete call says what is being searched or read (the start event only knows the tool's name).
            for block in blocks where block["type"] as? String == "tool_use" {
                let input = block["input"] as? [String: Any] ?? [:]
                let detail = (input["query"] as? String) ?? (input["url"] as? String) ?? ""
                if block["name"] as? String == "WebFetch", let id = block["id"] as? String, let url = input["url"] as? String {
                    fetchedURLs[id] = url
                }
                if let name = block["name"] as? String, !detail.isEmpty { events.append(.tool(name: name, summary: String(detail.prefix(160)))) }
            }
            return events

        case "user":
            guard let message = obj["message"] as? [String: Any], let blocks = message["content"] as? [[String: Any]] else { return [] }
            var events: [ProviderEvent] = []
            for block in blocks where block["type"] as? String == "tool_result" && block["is_error"] as? Bool != true {
                let links = Self.searchLinks(in: Self.text(of: block["content"]))
                if !links.isEmpty {
                    events += links.map { .source(title: $0.title, url: $0.url) }
                } else if let id = block["tool_use_id"] as? String, let url = fetchedURLs[id],
                          let source = ChatSource.make(title: "", url: url) {
                    events.append(.source(title: source.title, url: source.url))
                }
            }
            return events

        case "rate_limit_event":
            // Plan usage (5 h / weekly windows), learnt for free; nothing for the chat.
            if let info = obj["rate_limit_info"] as? [String: Any] { Limits.shared.recordClaude(info: info) }
            return []

        case "result":
            if obj["is_error"] as? Bool == true {
                let message = (obj["result"] as? String) ?? "Claude no pudo completar la respuesta."
                return [.failure(ProviderFailure(kind: ProviderFailure.classify(message), message: message))]
            }
            return [.done]

        default:
            return []
        }
    }

    // MARK: tool results

    private static func text(of content: Any?) -> String {
        if let text = content as? String { return text }
        if let blocks = content as? [[String: Any]] {
            return blocks.compactMap { $0["text"] as? String }.joined(separator: "\n")
        }
        return ""
    }

    /// A web search answers with `Links: [{"title":…,"url":…}, …]` followed by a summary.
    static func searchLinks(in text: String) -> [ChatSource] {
        guard let marker = text.range(of: "Links: [") else { return [] }
        let chars = Array(text[text.index(before: marker.upperBound)...])
        var depth = 0, inString = false, escaped = false, end: Int?
        for (i, c) in chars.enumerated() {
            if inString {
                if escaped { escaped = false } else if c == "\\" { escaped = true } else if c == "\"" { inString = false }
                continue
            }
            if c == "\"" { inString = true }
            else if c == "[" { depth += 1 }
            else if c == "]" { depth -= 1; if depth == 0 { end = i; break } }
        }
        guard let end, let data = String(chars[0...end]).data(using: .utf8),
              let items = (try? JSONSerialization.jsonObject(with: data)) as? [[String: Any]] else { return [] }
        var seen = Set<String>()
        return items.compactMap { item in
            guard let url = item["url"] as? String, let source = ChatSource.make(title: item["title"] as? String ?? "", url: url),
                  seen.insert(source.url).inserted else { return nil }
            return source
        }.prefix(10).map { $0 }
    }
}
