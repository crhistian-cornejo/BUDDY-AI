import Foundation

/// The model a turn runs on: which CLI, the id it receives, how it is shown to the user, and the reasoning effort (nil
/// where the model takes no effort flag: Haiku).
struct ModelChoice: Equatable, Sendable {
    var id: String        // what the CLI receives
    var display: String   // what the user sees
    var effort: String?   // low | medium | high | xhigh | max; nil: no effort flag
    var provider: ProviderID = .claude

    /// What the user sees: the model's name. The effort is internal and never shown.
    var summary: String { display }
}

/// The models are locked per agent (twin of `locked_providers`, `model_for` and `fallback_for` in
/// apps/windows/src-tauri/src/services/named_agents.rs). MIKA (`mika`) runs on Claude Sonnet 5.5 by default and is the only
/// agent that may be switched to Codex; EVERY other agent, built-in or added by the user, runs on Codex GPT-6.1-Sol. The
/// `provider:`, `providers:` and `model:` lines of an `agent.md` are ignored (a stale or hand-edited value can neither break
/// an agent nor override the lock); only MIKA's `provider:` is honoured. Effort is medium everywhere (PARLEY too: the
/// deeper efforts spent the subscription in a day) and never shown.
enum ModelCatalog {
    /// The orchestrator: the only agent with a provider choice.
    static let orchestrator = "mika"

    /// Claude Sonnet 5.5: MIKA's model and the fallback of every other agent.
    static let claude = ModelChoice(id: "claude-sonnet-5-5", display: "Sonnet 5.5", effort: "medium", provider: .claude)
    /// Codex GPT-6.1-Sol: the model of every agent but MIKA.
    static let codex = ModelChoice(id: "gpt-6.1-sol", display: "GPT-6.1-Sol", effort: "medium", provider: .codex)
    /// Claude Haiku 4.5: MIKA's fallback when Sonnet has no credits left (no effort flag).
    static let haiku = ModelChoice(id: "claude-haiku-4-5-20251001", display: "Haiku 4.5", effort: nil, provider: .claude)

    /// The providers an agent may use and the one it starts on. MIKA: Claude or Codex, Claude unless `preferred` (the
    /// file's `provider:`) says `codex`. Everyone else: Codex only, whatever the file says.
    static func lockedProviders(id: String, preferred: String?) -> (providers: [ProviderID], provider: ProviderID) {
        guard id == orchestrator else { return ([.codex], .codex) }
        let provider = preferred.flatMap { ProviderID(rawValue: $0) } ?? .claude
        return ([.claude, .codex], provider)
    }

    static func defaultChoice(for provider: ProviderID) -> ModelChoice {
        provider == .claude ? claude : codex
    }

    /// The model `agent` runs on `provider`. The agent's `model:` line is ignored. A turn retried on the fallback carries
    /// that model in `modelOverride` (never read from a file).
    static func choice(for agent: AgentDefinition, provider: ProviderID) -> ModelChoice {
        if let forced = agent.modelOverride, forced.provider == provider { return forced }
        return defaultChoice(for: provider)
    }

    /// The model a turn is retried on, once, when the primary has no credits left: Haiku 4.5 for MIKA, Sonnet 5.5 for the
    /// rest. It is never stored as a setting; the next turn tries the primary again.
    static func fallback(for agentID: String) -> ModelChoice {
        agentID == orchestrator ? haiku : claude
    }
}
