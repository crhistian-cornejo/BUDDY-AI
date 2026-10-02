import Foundation

// Skills of a named agent and the system prompt that carries them. A skill is a small markdown file the user wrote:
// `~/Library/Application Support/Mika/agents/<id>/skills/<slug>.md`, a front matter (`name`, `description`, `enabled`)
// and a body (the procedure). On every turn the prompt is assembled as: the agent's instructions (the body of its
// `agent.md`), a short identity line (its current name), then the ENABLED skills under `## Skills`; the caller adds the
// rest (the Telegram note, MIKA's team directory, the agent's memory). Skills are the user's own text, trusted like the
// agent prompt. The slug is the only thing that reaches a file path and it is validated first, so no path can leave the
// folder. Twin of apps/windows/src-tauri/src/services/agent_skills.rs (same limits, same messages).

struct AgentSkill: Equatable, Sendable, Identifiable {
    var slug: String
    var name: String
    var description: String
    var enabled: Bool
    var body: String
    var id: String { slug }
}

/// What the settings page sends. No `slug`: a new skill (its slug comes from the name); with one: that skill.
struct AgentSkillInput: Equatable, Sendable {
    var slug: String? = nil
    var name: String
    var description: String = ""
    var body: String
    /// nil: keep what it was (a new skill starts enabled).
    var enabled: Bool? = nil
}

/// The skills as the settings page shows them, with what the prompt budget leaves out.
struct AgentSkillsView: Equatable, Sendable {
    var skills: [AgentSkill]
    var maxSkills: Int
    var maxBody: Int
    var capChars: Int
    /// Characters the enabled skills that fit take in the prompt.
    var usedChars: Int
    /// Enabled skills (slugs) that did not fit under `capChars`: they are not sent to the agent.
    var dropped: [String]
}

enum AgentSkills {
    /// Skills an agent may have.
    static let maxSkills = 20
    /// Characters of one skill's body.
    static let maxBody = 8_000
    static let maxName = 60
    static let maxDescription = 240
    /// Characters of all the enabled skills together, as they go into the prompt. What does not fit is left out.
    static let maxTotal = 20_000
    /// A skill file bigger than this is ignored (the body cap is far below it).
    static let maxFileBytes: UInt64 = 64_000

    // MARK: Slugs

    /// `^[a-z0-9][a-z0-9-]{0,39}$`.
    static func validSlug(_ slug: String) -> Bool {
        let bytes = Array(slug.utf8)
        guard let first = bytes.first, bytes.count <= 40 else { return false }
        func lowerOrDigit(_ c: UInt8) -> Bool { (c >= 97 && c <= 122) || (c >= 48 && c <= 57) }
        guard lowerOrDigit(first) else { return false }
        return bytes.dropFirst().allSatisfy { lowerOrDigit($0) || $0 == UInt8(ascii: "-") }
    }

    /// A file name from a name: ASCII lowercase, accents dropped, anything else a dash.
    static func slugify(_ name: String) -> String {
        var out = ""
        for scalar in name.trimmingCharacters(in: .whitespacesAndNewlines).lowercased().unicodeScalars {
            let mapped: Unicode.Scalar
            switch scalar {
            case "á", "à", "ä", "â": mapped = "a"
            case "é", "è", "ë", "ê": mapped = "e"
            case "í", "ì", "ï", "î": mapped = "i"
            case "ó", "ò", "ö", "ô": mapped = "o"
            case "ú", "ù", "ü", "û": mapped = "u"
            case "ñ": mapped = "n"
            default: mapped = scalar
            }
            if (mapped.value >= 97 && mapped.value <= 122) || (mapped.value >= 48 && mapped.value <= 57) {
                out.unicodeScalars.append(mapped)
            } else if !out.hasSuffix("-") && !out.isEmpty {
                out.append("-")
            }
        }
        var trimmed = String(out.prefix(40))
        while trimmed.hasSuffix("-") { trimmed.removeLast() }
        return trimmed.isEmpty ? "skill" : trimmed
    }

    // MARK: Validation

    private static func capital(_ text: String) -> String {
        guard let first = text.first else { return "" }
        return first.uppercased() + text.dropFirst()
    }

    /// A one-line field: trimmed, within `max` characters, no control characters or double quotes.
    private static func oneLine(_ raw: String, max: Int, what: String, required: Bool) throws -> String {
        let text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        let count = text.unicodeScalars.count
        if required && count == 0 { throw AgentExtrasError("Falta \(what).") }
        if count > max { throw AgentExtrasError("\(capital(what)) pasa de \(max) caracteres.") }
        if text.unicodeScalars.contains(where: { $0.properties.generalCategory == .control || $0 == "\"" }) {
            throw AgentExtrasError("\(capital(what)) no puede tener saltos de línea ni comillas dobles.")
        }
        return text
    }

    private static func cleanBody(_ raw: String) throws -> String {
        let body = raw.replacingOccurrences(of: "\r\n", with: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
        if body.isEmpty { throw AgentExtrasError("Falta el procedimiento (el cuerpo de la skill).") }
        let count = body.unicodeScalars.count
        if count > maxBody { throw AgentExtrasError("El cuerpo pasa de \(maxBody) caracteres (\(count)).") }
        return body
    }

    // MARK: File format

    /// The text of the file.
    static func renderFile(_ skill: AgentSkill) -> String {
        "---\nname: \"\(skill.name)\"\ndescription: \"\(skill.description)\"\nenabled: \(skill.enabled)\n---\n\(skill.body)\n"
    }

    static func parseSkill(slug: String, text: String) -> AgentSkill? {
        let lines = text.replacingOccurrences(of: "\r\n", with: "\n").components(separatedBy: "\n")
        guard lines.first?.trimmingCharacters(in: .whitespaces) == "---" else { return nil }
        var name = "", description = "", enabled = true
        var closedAt: Int?
        for index in 1..<lines.count {
            let line = lines[index]
            if line.trimmingCharacters(in: .whitespaces) == "---" { closedAt = index; break }
            guard let colon = line.firstIndex(of: ":") else { continue }
            let key = line[..<colon].trimmingCharacters(in: .whitespaces)
            var value = line[line.index(after: colon)...].trimmingCharacters(in: .whitespaces)
            if value.count >= 2, value.hasPrefix("\""), value.hasSuffix("\"") { value = String(value.dropFirst().dropLast()) }
            switch key {
            case "name": name = value
            case "description": description = value
            case "enabled": enabled = value != "false"
            default: break
            }
        }
        guard let closedAt, !name.isEmpty else { return nil }
        let body = lines[(closedAt + 1)...].joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
        return AgentSkill(slug: slug, name: name, description: description, enabled: enabled, body: body)
    }

    // MARK: Files

    /// The skills folder of an agent. The id is validated first: no path is ever built from unchecked text.
    static func directory(agent id: String, root: URL) throws -> URL {
        guard AgentLookRules.validID(id) else { throw AgentExtrasError("Agente no válido.") }
        return root.appendingPathComponent(id, isDirectory: true).appendingPathComponent("skills", isDirectory: true)
    }

    private static func file(in dir: URL, slug: String) throws -> URL {
        guard validSlug(slug) else { throw AgentExtrasError("Nombre de archivo no válido.") }
        return dir.appendingPathComponent("\(slug).md", isDirectory: false)
    }

    /// Every readable skill of the folder, by slug. A damaged, oversized, linked or oddly named file is skipped.
    static func list(in dir: URL) -> [AgentSkill] {
        let fm = FileManager.default
        guard let names = try? fm.contentsOfDirectory(atPath: dir.path) else { return [] }
        var skills: [AgentSkill] = []
        for fileName in names {
            guard fileName.hasSuffix(".md") else { continue }
            let slug = String(fileName.dropLast(3))
            guard validSlug(slug) else { continue }
            let path = dir.appendingPathComponent(fileName).path
            // attributesOfItem does not follow links: a symlink is `.typeSymbolicLink` and is skipped.
            guard let attributes = try? fm.attributesOfItem(atPath: path),
                  attributes[.type] as? FileAttributeType == .typeRegular,
                  let size = (attributes[.size] as? NSNumber)?.uint64Value, size <= maxFileBytes,
                  let text = try? String(contentsOfFile: path, encoding: .utf8),
                  let skill = parseSkill(slug: slug, text: text) else { continue }
            skills.append(skill)
        }
        return skills.sorted { $0.slug < $1.slug }
    }

    static func get(in dir: URL, slug: String) throws -> AgentSkill {
        _ = try file(in: dir, slug: slug)
        guard let skill = list(in: dir).first(where: { $0.slug == slug }) else { throw AgentExtrasError("Esa skill no existe.") }
        return skill
    }

    /// Creates or updates a skill. Validates everything before writing; the write is atomic.
    @discardableResult
    static func save(in dir: URL, _ input: AgentSkillInput) throws -> AgentSkill {
        let name = try oneLine(input.name, max: maxName, what: "el nombre", required: true)
        let description = try oneLine(input.description, max: maxDescription, what: "la descripción", required: false)
        let body = try cleanBody(input.body)
        let existing = list(in: dir)
        let slug: String
        if let given = input.slug {
            guard validSlug(given) else { throw AgentExtrasError("Nombre de archivo no válido.") }
            slug = given
        } else {
            let base = slugify(name)
            var candidate = base
            var n = 2
            while existing.contains(where: { $0.slug == candidate }) {
                let suffix = "-\(n)"
                var stem = String(base.prefix(40 - suffix.count))
                while stem.hasSuffix("-") { stem.removeLast() }
                candidate = stem + suffix
                n += 1
            }
            slug = candidate
        }
        let previous = existing.first { $0.slug == slug }
        if previous == nil && existing.count >= maxSkills {
            throw AgentExtrasError("Cada agente puede tener hasta \(maxSkills) skills.")
        }
        let skill = AgentSkill(slug: slug, name: name, description: description,
                               enabled: input.enabled ?? previous?.enabled ?? true, body: body)
        try write(skill, in: dir)
        return skill
    }

    private static func write(_ skill: AgentSkill, in dir: URL) throws {
        let target = try file(in: dir, slug: skill.slug)
        do {
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true,
                                                    attributes: [.posixPermissions: 0o700])
        } catch { throw AgentExtrasError("No se pudo crear la carpeta de skills.") }
        do { try renderFile(skill).write(to: target, atomically: true, encoding: .utf8) }
        catch { throw AgentExtrasError("No se pudo guardar la skill.") }
    }

    static func delete(in dir: URL, slug: String) throws {
        let target = try file(in: dir, slug: slug)
        guard FileManager.default.fileExists(atPath: target.path) else { throw AgentExtrasError("Esa skill no existe.") }
        do { try FileManager.default.removeItem(at: target) } catch { throw AgentExtrasError("No se pudo borrar la skill.") }
    }

    @discardableResult
    static func toggle(in dir: URL, slug: String, enabled: Bool) throws -> AgentSkill {
        var skill = try get(in: dir, slug: slug)
        skill.enabled = enabled
        try write(skill, in: dir)
        return skill
    }

    // MARK: The prompt

    /// One skill as the agent reads it.
    static func block(_ skill: AgentSkill) -> String {
        let description = skill.description.isEmpty ? "" : skill.description + "\n\n"
        return "### \(skill.name)\n\(description)\(skill.body)\n\n"
    }

    /// The enabled skills that fit under `maxTotal`, in slug order, the slugs of the enabled ones that do not, and the
    /// characters the kept ones take.
    static func select(_ skills: [AgentSkill]) -> (kept: [AgentSkill], dropped: [String], used: Int) {
        var used = 0
        var kept: [AgentSkill] = []
        var dropped: [String] = []
        for skill in skills where skill.enabled {
            let size = block(skill).unicodeScalars.count
            if used + size <= maxTotal {
                used += size
                kept.append(skill)
            } else {
                dropped.append(skill.slug)
            }
        }
        return (kept, dropped, used)
    }

    /// `## Skills` and the skills that fit; empty when there are none.
    static func section(_ skills: [AgentSkill]) -> String {
        let kept = select(skills).kept
        if kept.isEmpty { return "" }
        var out = "\n\n## Skills\nProcedimientos que el usuario te enseñó. Aplica el que corresponda cuando la petición encaje con su descripción.\n\n"
        for skill in kept { out += block(skill) }
        while let last = out.last, last.isWhitespace { out.removeLast() }
        return out
    }

    /// Instructions, then identity, then skills.
    static func compose(instructions: String, name: String, id: String, skills: [AgentSkill]) -> String {
        instructions + AgentPrompt.identityLine(name: name, id: id) + section(skills)
    }

    static func view(in dir: URL) -> AgentSkillsView {
        let skills = list(in: dir)
        let chosen = select(skills)
        return AgentSkillsView(skills: skills, maxSkills: maxSkills, maxBody: maxBody, capChars: maxTotal,
                               usedChars: chosen.used, dropped: chosen.dropped)
    }
}
