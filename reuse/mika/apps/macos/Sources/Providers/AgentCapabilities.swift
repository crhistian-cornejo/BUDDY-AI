import Foundation

// What a named agent may do: the `can: [read, run, images, pdf, web, edit]` line of its agent.md. Twin of the
// capabilities section of apps/windows/src-tauri/src/services/named_agents.rs and of apps/windows/src/core/capabilities.ts:
// the same closed set, the same order, the same dependencies and the same hard rule.
//
//  * A file without a `can:` line (agents saved before it existed) gets it derived from `access`, `accepts` and `tools`:
//    read always; edit for `access: edit`; images / pdf from `accepts`; web from `tools`; `run` never. A file with none
//    of those keys (an agent the user added) gets read and web.
//  * When `can:` exists, `access`, `accepts` and `tools` are ignored: the parsed agent derives them back from `can`.
//  * `edit`, `images` and `pdf` need `read` (the agent opens the file it edits or is given): `read` is added.
//  * Hard rule: an agent fed by Telegram (PARLEY, or any agent with `integration: telegram`) reads strangers' text, so
//    `run` and `edit` are removed from it whatever the file says, and refused when someone tries to set them.

enum AgentCapability: String, CaseIterable, Sendable {
    case read, run, images, pdf, web, edit, office

    var label: String {
        switch self {
        case .read: return "Leer archivos"
        case .run: return "Ejecutar comandos"
        case .images: return "Ver imágenes"
        case .pdf: return "Leer PDF"
        case .web: return "Web"
        case .edit: return "Editar archivos"
        case .office: return "Crear Word, Excel y PowerPoint"
        }
    }

    /// One line, under the switch.
    var detail: String {
        switch self {
        case .read: return "Abre los archivos de su carpeta de trabajo."
        case .run: return "Corre comandos en su carpeta de trabajo. Cada uno te pide permiso en la isla."
        case .images: return "Mira las imágenes que le sueltas en el chat."
        case .pdf: return "Lee los PDF que le sueltas en el chat."
        case .web: return "Busca y lee páginas de internet."
        case .edit: return "Crea y cambia archivos dentro de su carpeta de trabajo."
        case .office: return "Escribe .docx, .xlsx y .pptx en su carpeta documentos. Solo funciona en agentes que corren en Codex."
        }
    }

    /// The switches Settings shows.
    static let switchable: [AgentCapability] = [.read, .run, .images, .pdf, .web, .edit, .office]
}

enum AgentCapabilities {
    /// Why a switch is off and locked (shown next to it).
    static let forbiddenReason = "Lee mensajes de desconocidos de Telegram: no puede ejecutar comandos ni editar archivos."

    /// What an agent can never have.
    static func forbidden(id: String, integration: String?) -> Set<AgentCapability> {
        id == Picks.agentID || integration == TelegramInbox.agentIntegration ? [.run, .edit, .office] : []
    }

    /// An agent fed by Telegram never gets connectors: a stranger's message could try to steer it into the user's mail.
    static func forbidsConnectors(id: String, integration: String?) -> Bool {
        id == Picks.agentID || integration == TelegramInbox.agentIntegration
    }

    /// `caps` as the closed set: no repeats, in canonical order, without what the agent can never have, with the
    /// dependencies added.
    static func normalize(id: String, integration: String?, _ caps: [AgentCapability]) -> [AgentCapability] {
        var on = Set(caps).subtracting(forbidden(id: id, integration: integration))
        if !on.isDisjoint(with: [.edit, .images, .pdf]) { on.insert(.read) }
        return AgentCapability.allCases.filter { on.contains($0) }
    }

    /// `can` from the keys that came before it. `anyLegacy`: the file had `access`, `accepts` or `tools`.
    static func derive(anyLegacy: Bool, access: AgentAccess, accepts: [String], tools: [String]) -> [AgentCapability] {
        guard anyLegacy else { return [.read, .web] }
        var caps: [AgentCapability] = [.read]
        if access == .edit { caps.append(.edit) }
        if accepts.contains("image") { caps.append(.images) }
        if accepts.contains("pdf") { caps.append(.pdf) }
        if tools.contains("web") { caps.append(.web) }
        // MAKI (files) and MIRA (docs) always wrote documents: they get the Office tools; the rest do not.
        if tools.contains("docs") || tools.contains("files") { caps.append(.office) }
        return caps
    }

    /// What the chat takes in: text, and pictures / PDF when the agent can see them.
    static func accepts(of can: [AgentCapability]) -> [String] {
        ["text"] + (can.contains(.images) ? ["image"] : []) + (can.contains(.pdf) ? ["pdf"] : [])
    }

    /// The result of flipping one switch: turning `read` off also turns off what needs it; turning one of those on
    /// turns `read` on. A forbidden one never turns on.
    static func toggle(_ current: [AgentCapability], _ cap: AgentCapability, on: Bool, id: String, integration: String?) -> [AgentCapability] {
        var next = Set(current)
        if on { next.insert(cap) } else {
            next.remove(cap)
            if cap == .read { next.subtract([.edit, .images, .pdf]) }
        }
        return normalize(id: id, integration: integration, AgentCapability.allCases.filter { next.contains($0) })
    }

    /// What Settings asked for, checked: a forbidden capability is an error (nothing is written).
    static func validateEdit(id: String, integration: String?, _ caps: [AgentCapability]) throws -> [AgentCapability] {
        if !Set(caps).isDisjoint(with: forbidden(id: id, integration: integration)) { throw AgentLookError.forbiddenCapability }
        return normalize(id: id, integration: integration, caps)
    }

    /// The `can:` line as the file holds it.
    static func line(_ caps: [AgentCapability]) -> String { "can: [" + caps.map(\.rawValue).joined(separator: ", ") + "]" }

    /// The tools of the `claude` CLI an agent can use without asking. A command tool is never one of them, whatever
    /// `can` says: running commands goes through the approval gate (`ClaudeCommand` adds Bash to `--tools` only then).
    static func claudeTools(for can: [AgentCapability]) -> [String] {
        var tools: [String] = []
        if can.contains(.read) { tools += ["Read", "Grep", "Glob"] }
        if can.contains(.web) { tools.append("WebSearch") }
        if can.contains(.edit) { tools += ["Write", "Edit"] }
        return tools
    }
}
