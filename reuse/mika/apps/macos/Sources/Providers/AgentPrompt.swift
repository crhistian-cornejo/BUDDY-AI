import Foundation

// The prompt of a named agent: the body of its `agent.md`. Pure helpers to rename the agent inside it, to replace it
// without touching the front matter, and the cap. Twin of `replace_word`, `rename_in_body`, `replace_body` and
// `MAX_INSTRUCTIONS` in apps/windows/src-tauri/src/services/named_agents.rs. How a turn's system prompt is assembled
// (instructions, identity line, skills) is in AgentSkills.swift.

/// A message for the user about instructions or skills (shown inline in Settings). Equatable so tests can compare.
struct AgentExtrasError: Error, Equatable, LocalizedError {
    let message: String
    init(_ message: String) { self.message = message }
    var errorDescription: String? { message }
}

/// The prompt of an agent as the settings page edits it.
struct AgentInstructions: Equatable, Sendable {
    var text: String
    var max: Int
    /// A built-in: it has shipped instructions to go back to.
    var builtin: Bool
    /// A built-in whose instructions differ from the shipped ones (with the agent's own name).
    var modified: Bool
}

enum AgentPrompt {
    /// The most characters the instructions of an agent may have.
    static let maxInstructions = 12_000

    private static func isWord(_ scalar: Unicode.Scalar) -> Bool {
        CharacterSet.alphanumerics.contains(scalar) || scalar == "_"
    }

    /// `text` with every whole-word, case-sensitive `old` replaced by `new` ("MIRA" in "Eres MIRA," but not in "MIRAR").
    static func replaceWord(_ text: String, old: String, new: String) -> String {
        guard !old.isEmpty, old != new else { return text }
        let source = Array(text.unicodeScalars)
        let needle = Array(old.unicodeScalars)
        let firstIsWord = needle.first.map(isWord) ?? false
        let lastIsWord = needle.last.map(isWord) ?? false
        var out = String.UnicodeScalarView()
        var index = 0
        while index < source.count {
            if index + needle.count <= source.count, source[index..<(index + needle.count)].elementsEqual(needle) {
                let before: Unicode.Scalar? = index > 0 ? source[index - 1] : nil
                let after: Unicode.Scalar? = index + needle.count < source.count ? source[index + needle.count] : nil
                let blocked = (firstIsWord && before.map(isWord) == true) || (lastIsWord && after.map(isWord) == true)
                if !blocked {
                    out.append(contentsOf: new.unicodeScalars)
                    index += needle.count
                    continue
                }
            }
            out.append(source[index])
            index += 1
        }
        return String(out)
    }

    /// The front matter and the prompt of an `agent.md`: `head` ends with the closing `---` line and its line ending.
    /// Nil when the file has no front matter. A file whose front matter is the whole file has an empty `body`.
    static func split(_ text: String) -> (head: String, body: String)? {
        let lines = text.components(separatedBy: "\n")
        guard lines.first?.trimmingCharacters(in: .whitespacesAndNewlines) == "---",
              let end = lines.indices.dropFirst().first(where: {
                  lines[$0].trimmingCharacters(in: .whitespacesAndNewlines) == "---"
              })
        else { return nil }
        let head = lines[0...end].joined(separator: "\n") + (end + 1 < lines.count ? "\n" : "")
        let body = end + 1 < lines.count ? lines[(end + 1)...].joined(separator: "\n") : ""
        return (head, body)
    }

    /// The same file with `old` replaced by `new` in the prompt (the text after the front matter) only.
    static func renameInBody(_ text: String, old: String, new: String) -> String {
        guard let parts = split(text) else { return text }
        return parts.head + replaceWord(parts.body, old: old, new: new)
    }

    /// The file with another prompt after the same front matter. The line endings of the file are kept; the prompt is
    /// trimmed, must not be empty and must fit in `maxInstructions` characters.
    static func replaceBody(_ text: String, with raw: String) throws -> String {
        let normalized = raw.replacingOccurrences(of: "\r\n", with: "\n")
        let body = normalized.trimmingCharacters(in: .whitespacesAndNewlines)
        if body.isEmpty { throw AgentExtrasError("Las instrucciones no pueden estar vacías.") }
        let count = body.unicodeScalars.count
        if count > maxInstructions {
            throw AgentExtrasError("Las instrucciones pasan de \(maxInstructions) caracteres (\(count)).")
        }
        guard let parts = split(text) else { throw AgentLookError.missingFrontmatter }
        let crlf = parts.head.contains("\r\n")
        let eol = crlf ? "\r\n" : "\n"
        let written = crlf ? body.replacingOccurrences(of: "\n", with: "\r\n") : body
        // A head that was the last thing in the file has no line ending yet.
        let head = parts.head.utf8.last == UInt8(ascii: "\n") ? parts.head : parts.head + eol
        return head + written + eol
    }

    /// The identity line every turn carries: the name the user gave the agent now.
    static func identityLine(name: String, id: String) -> String {
        "\n\nTu nombre en MIKA es \(name) (id \(id)); los otros agentes y el usuario te llaman así."
    }
}
