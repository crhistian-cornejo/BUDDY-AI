import Foundation

/// The one short line that says what an agent is doing while it works ("Buscando: alianza lima hoy…").
enum ToolStatus {
    static func label(name: String, summary: String) -> String {
        let key = name.lowercased().replacingOccurrences(of: "_", with: "")
        let detail = summary.trimmingCharacters(in: .whitespacesAndNewlines)
        switch key {
        case "websearch":
            return detail.isEmpty ? "Buscando en la web…" : "Buscando: \(cut(detail, 42))…"
        case "webfetch":
            if let host = ChatSource.host(of: detail) { return "Leyendo \(host)…" }
            return "Leyendo una página…"
        case "bash", "commandexecution": return "Ejecutando un comando…"
        case "read": return "Leyendo archivos…"
        case "write", "edit", "multiedit", "notebookedit", "filechange": return "Editando archivos…"
        case "grep", "glob": return "Buscando en archivos…"
        default: return "Trabajando…"
        }
    }

    private static func cut(_ text: String, _ limit: Int) -> String {
        guard text.count > limit else { return text }
        return String(text.prefix(limit)).trimmingCharacters(in: .whitespaces) + "…"
    }
}
