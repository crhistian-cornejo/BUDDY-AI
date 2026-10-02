import Foundation

/// A failure already in the user's words (Spanish, one line), shown as is: the Telegram helper's answers, PARLEY's
/// reviews, an automatic turn that could not run. Never carries a secret or a raw CLI log.
struct MessageError: LocalizedError, Equatable, Sendable {
    let message: String

    init(_ message: String) { self.message = message }

    var errorDescription: String? { message }

    /// The text to show for any error: its own words when it is one of these, a generic line otherwise.
    static func text(_ error: Error, fallback: String = "Algo salió mal. Intenta de nuevo.") -> String {
        if let known = error as? MessageError { return known.message }
        if let failure = error as? ProviderFailure { return failure.userSummary }
        return fallback
    }
}
