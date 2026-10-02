import Foundation

enum ProviderID: String, Codable, CaseIterable, Sendable { case claude, codex }

enum ProviderFailureKind: String, Equatable, Sendable { case auth, limit, timeout, cli, other }

struct ProviderFailure: Error, Equatable, Sendable {
    var kind: ProviderFailureKind
    var message: String
    var resetsAt: Date? = nil

    /// One short line for the chat header. A limit says when it resets whenever the provider told us.
    var userSummary: String {
        // A per-minute rate limit or a busy service is temporary: it is not "no credits left" (and is never retried).
        if CreditFallback.isTooManyRequests(message) { return "Demasiadas peticiones · intenta de nuevo en un momento" }
        // An old Codex client does not know the model: updating it (`codex update`) is the whole fix.
        if message.lowercased().contains("model is not supported") { return "Actualiza Codex: ejecuta «codex update»" }
        switch kind {
        case .limit:
            if let range = message.range(of: "resets", options: .caseInsensitive) {
                let when = message[range.upperBound...].trimmingCharacters(in: CharacterSet(charactersIn: " .·:\n"))
                if !when.isEmpty { return "Límite de uso · reinicia \(when)" }
            }
            let detail = message.trimmingCharacters(in: .whitespacesAndNewlines)
            return detail.isEmpty ? "Límite de uso alcanzado" : "Límite de uso · " + String(detail.prefix(90))
        case .auth: return "Conecta tu cuenta"
        case .timeout: return "Tardó demasiado"
        case .cli: return "No se pudo iniciar el cliente"
        case .other: return "No se pudo responder"
        }
    }

    /// The account has no credits or usage left (the only failure the turn is retried on the fallback model for).
    var isNoCredits: Bool { CreditFallback.isNoCredits(message) }

    /// Classifies a CLI error text. Order matters: a limit message can also mention "login".
    static func classify(_ text: String) -> ProviderFailureKind {
        let t = text.lowercased()
        if t.contains("limit") || t.contains("quota") { return .limit }
        if t.contains("not logged in") || t.contains("/login") || t.contains("unauthorized")
            || t.contains("authenticat") || t.contains("sign in") || t.contains("log in") { return .auth }
        return .other
    }
}

enum ProviderEvent: Equatable, Sendable {
    case session(String)
    case delta(String)
    case progress(String)
    case tool(name: String, summary: String)
    case source(title: String, url: String)   // a page the answer used (shown as an icon under the answer)
    case imageStarted                // the agent began creating an image (shows the skeleton)
    case image(path: String)
    case imageFailed(String)         // the image could not be created (readable reason)
    case done
    case failure(ProviderFailure)
}
