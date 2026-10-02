import Foundation

// What a chat may be given as a file. Text goes into the prompt as data; a picture or a PDF is copied into the agent's
// workspace and the agent opens it with its own tools, and only when the agent can: `can: [images]` / `can: [pdf]`.
// An agent that cannot says so in one line and points to the specialist (MIRA).

enum AttachmentKind: Equatable, Sendable {
    case text, image, pdf
}

enum AttachmentPolicy {
    /// The agent that reads documents and looks at pictures; the others point to it.
    static let specialistID = "mira"

    static func kind(of url: URL) -> AttachmentKind {
        let ext = url.pathExtension.lowercased()
        if ChatImagePolicy.extensions.contains(ext) { return .image }
        if ext == "pdf" { return .pdf }
        return .text
    }

    /// Why a file cannot go to this agent, in one short line, and whether the specialist can take it.
    struct Block: Equatable, Sendable {
        var message: String
        var offersSpecialist: Bool
    }

    /// Nil when the agent can take the file. `specialistName` is nil when there is no specialist to suggest (not installed,
    /// or it is this very agent).
    static func block(kind: AttachmentKind, can: [AgentCapability], provider: ProviderID, agentName: String,
                      specialistName: String?) -> Block? {
        let need: AgentCapability
        let what: String
        switch kind {
        case .text: return nil
        case .image: need = .images; what = "imágenes"
        case .pdf: need = .pdf; what = "PDF"
        }
        let alternative = specialistName.map { " o usa a \($0)" } ?? ""
        if !can.contains(need) {
            return Block(message: "\(agentName) no lee \(what): actívalo en sus ajustes\(alternative).",
                         offersSpecialist: specialistName != nil)
        }
        // Codex gets pictures attached to the turn but has no tool that opens a PDF.
        if kind == .pdf && provider == .codex {
            return Block(message: "Con Codex no se abren PDF: usa un agente con Claude\(alternative).",
                         offersSpecialist: specialistName != nil)
        }
        return nil
    }

    /// Too big for the provider (`FileLimits`), or not a regular file.
    static func sizeProblem(kind: AttachmentKind, url: URL) -> String? {
        guard let size = FileLimits.regularFileSize(url) else { return "Solo se pueden adjuntar archivos, no carpetas." }
        switch kind {
        case .image where size > FileLimits.maxImageBytes: return "La imagen pesa más de 5 MB."
        case .pdf where size > FileLimits.maxPDFBytes: return "El PDF pesa más de 10 MB."
        default: return nil
        }
    }

    /// A copy of the file inside the agent's workspace (its working folder), which is where its tools may read. The name
    /// is cleaned so it cannot climb out of the folder; an older copy with the same name is replaced.
    static func stage(_ url: URL, in workspace: URL) -> URL? {
        let folder = workspace.appendingPathComponent("adjuntos", isDirectory: true)
        let fm = FileManager.default
        guard (try? fm.createDirectory(at: folder, withIntermediateDirectories: true,
                                       attributes: [.posixPermissions: 0o700])) != nil else { return nil }
        let dest = folder.appendingPathComponent(safeName(url.lastPathComponent))
        try? fm.removeItem(at: dest)
        guard (try? fm.copyItem(at: url.resolvingSymlinksInPath(), to: dest)) != nil else { return nil }
        return dest
    }

    static func safeName(_ name: String) -> String {
        let cleaned = name.map { $0.isLetter || $0.isNumber || ".-_ ()".contains($0) ? $0 : "_" }
        let text = String(cleaned).trimmingCharacters(in: .whitespaces)
        let trimmed = text.hasPrefix(".") ? "_" + text.dropFirst() : text
        return trimmed.isEmpty ? "archivo" : String(trimmed.prefix(120))
    }
}
