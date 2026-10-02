import Foundation

/// When an agent's primary model has no credits left, the turn is retried once on its fallback model (`ModelCatalog.fallback`).
/// Only a no-credits error does that: a rate limit, a context limit, a login problem, a timeout or a network failure are
/// shown as they always were. Twin of `is_usage_limit`, `fallback_notice` and `FALLBACK_KEY` in
/// apps/windows/src-tauri/src/services/subscription.rs.
enum CreditFallback {
    /// The key prefix of the session of a turn retried on the fallback model (kept apart from the primary's).
    static let keyPrefix = "fb-"

    /// Lower-case fragments of the provider's error text that mean the account has no credits or usage left (Claude's
    /// "Usage limit reached" and "5-hour / weekly limit reached", Codex's "Usage limit" / `usageLimitExceeded`, quota and
    /// credit-balance errors).
    static let noCreditsKeywords: [String] = [
        "usage limit", "usagelimitexceeded", "usage_limit", "limit reached", "reached your limit", "hit your limit",
        "hit your usage", "quota", "out of credits", "insufficient credits", "no credits", "credit balance",
        "credits are exhausted", "run out of credits",
    ]

    /// Whether a provider's error text says the account has no credits or usage left. A per-minute rate limit (429) is
    /// temporary, not an empty account: anything that mentions `rate limit` / `rate_limit` without `usage limit` is not one.
    static func isNoCredits(_ detail: String) -> Bool {
        let lower = detail.lowercased()
        if (lower.contains("rate limit") || lower.contains("rate_limit")) && !lower.contains("usage limit") { return false }
        return noCreditsKeywords.contains { lower.contains($0) }
    }

    /// Too many requests at once, or the service is busy: temporary, never retried on the fallback.
    static func isTooManyRequests(_ detail: String) -> Bool {
        if isNoCredits(detail) { return false }
        let lower = detail.lowercased()
        return lower.contains("rate limit") || lower.contains("rate_limit") || lower.contains("too many requests")
            || lower.contains("overloaded")
    }

    /// The line shown in the chat while a turn is answered by the fallback model.
    static func notice(primary: String, fallback: String) -> String {
        "Sin créditos de \(primary): respondo con \(fallback)"
    }
}
