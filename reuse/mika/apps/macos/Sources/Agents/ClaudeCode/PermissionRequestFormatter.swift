import Foundation

/// Turns a Claude Code PermissionRequest payload into what the approval card shows:
/// the tool, its primary arguments, the full command / input (never truncated here) and the
/// text of the permission rules that "Always" would save.
enum PermissionRequestFormatter {

    /// Primary arguments, shown as "label  value" rows above the command block.
    private static let primaryKeys: [(key: String, label: String)] = [
        ("file_path", "File"),
        ("notebook_path", "Notebook"),
        ("path", "Path"),
        ("url", "URL"),
        ("pattern", "Pattern"),
        ("glob", "Glob"),
        ("query", "Query"),
        ("description", "Description"),
        ("subagent_type", "Agent"),
    ]

    static func approvalInfo(sessionId: String, payload: [String: Any]) -> ApprovalInfo {
        let toolName = payload["tool_name"] as? String ?? "Tool"
        let input = payload["tool_input"] as? [String: Any] ?? [:]

        var fields: [ApprovalField] = []
        var displayTool = toolName

        // MCP tools are named "mcp__<server>__<tool>".
        if toolName.hasPrefix("mcp__") {
            let parts = toolName.components(separatedBy: "__")
            displayTool = "MCP"
            if parts.count >= 2 { fields.append(ApprovalField(label: "Server", value: parts[1])) }
            if parts.count >= 3 {
                fields.append(ApprovalField(label: "Tool", value: parts[2...].joined(separator: "__")))
            }
        }

        var shownKeys = Set<String>()
        for (key, label) in primaryKeys {
            guard let value = input[key] as? String, !value.isEmpty else { continue }
            fields.append(ApprovalField(label: label, value: value))
            shownKeys.insert(key)
        }

        // Main block: the full shell command when there is one, then every other argument,
        // so nothing the user approves stays hidden.
        var sections: [String] = []
        if let command = input["command"] as? String {
            sections.append(command)
            shownKeys.insert("command")
        }
        for key in input.keys.sorted() where !shownKeys.contains(key) {
            guard let value = input[key] else { continue }
            if let text = value as? String {
                sections.append("\(key):\n\(text)")
            } else {
                sections.append("\(key): \(jsonText(value, pretty: true))")
            }
        }

        let suggestions = payload["permission_suggestions"] as? [[String: Any]] ?? []
        let rules = suggestions.map { describe(suggestion: $0) }

        return ApprovalInfo(sessionId: sessionId,
                            tool: displayTool,
                            fields: fields,
                            command: sections.joined(separator: "\n\n"),
                            rules: rules)
    }

    // MARK: - "Always" rule text

    /// One readable line per suggested permission update, e.g.
    /// "allow Bash(npm test:*) · this project (local settings)".
    static func describe(suggestion: [String: Any]) -> String {
        let type = suggestion["type"] as? String ?? ""
        let destination = describe(destination: suggestion["destination"] as? String)
        switch type {
        case "addRules", "replaceRules", "removeRules":
            let behavior = suggestion["behavior"] as? String ?? "allow"
            let rules = (suggestion["rules"] as? [[String: Any]] ?? []).map { rule -> String in
                let name = rule["toolName"] as? String ?? "?"
                if let content = rule["ruleContent"] as? String, !content.isEmpty {
                    return "\(name)(\(content))"
                }
                return name
            }
            let verb: String
            switch type {
            case "replaceRules": verb = "replace \(behavior) rules with"
            case "removeRules":  verb = "remove \(behavior) rule"
            default:             verb = behavior
            }
            return "\(verb) \(rules.joined(separator: ", "))\(destination)"
        case "setMode":
            return "switch permission mode to \(suggestion["mode"] as? String ?? "?")\(destination)"
        case "addDirectories", "removeDirectories":
            let dirs = (suggestion["directories"] as? [String] ?? []).joined(separator: ", ")
            let verb = type == "addDirectories" ? "allow access to" : "remove access to"
            return "\(verb) \(dirs)\(destination)"
        default:
            return jsonText(suggestion, pretty: false)
        }
    }

    private static func describe(destination: String?) -> String {
        guard let destination, !destination.isEmpty else { return "" }
        switch destination {
        case "localSettings":   return " · this project (local settings)"
        case "projectSettings": return " · this project (shared settings)"
        case "userSettings":    return " · all your projects"
        case "session":         return " · this session only"
        default:                return " · \(destination)"
        }
    }

    // MARK: - JSON text

    /// JSON text for any value parsed by JSONSerialization (slashes kept readable).
    private static func jsonText(_ value: Any, pretty: Bool) -> String {
        // Wrapping in an array lets isValidJSONObject vouch for scalars too.
        guard JSONSerialization.isValidJSONObject([value]) else { return String(describing: value) }
        var options: JSONSerialization.WritingOptions = [.sortedKeys, .withoutEscapingSlashes, .fragmentsAllowed]
        if pretty { options.insert(.prettyPrinted) }
        guard let data = try? JSONSerialization.data(withJSONObject: value, options: options),
              let text = String(data: data, encoding: .utf8) else { return String(describing: value) }
        return text
    }
}
