import Foundation

/// Shows an agent's command on the island's approval card and waits for the click. Only a pointer click on Allow allows
/// (the card ignores the keyboard); Deny, a timeout, Stop or anything else is a refusal. One card at a time: when another
/// approval is up, the request is not shown and `AgentGate.ask` tries again, then refuses.
@MainActor
final class IslandGatePresenter: GatePresenter {
    static let shared = IslandGatePresenter()
    /// How long a card waits for a click.
    static let cardTimeout: TimeInterval = 110

    private var continuation: CheckedContinuation<GateAnswer, Never>?
    private var requestId: UUID?
    private var requestAgent = ""
    /// How long a card has been waiting for the test to drive it (see `waitingFor`).

    var isPending: Bool { continuation != nil }

    nonisolated func ask(turn: GateTurn, tool: String, command: String, description: String?, notes: [String]) async -> GateAnswer {
        await present(turn: turn, tool: tool, command: command, description: description, notes: notes)
    }

    private func present(turn: GateTurn, tool: String, command: String, description: String?, notes: [String]) async -> GateAnswer {
        let state = AppState.shared
        guard continuation == nil, state.pendingApproval == nil else { return .notShown }
        var info = ApprovalInfo(sessionId: "agent-\(turn.agentId)", tool: tool, command: command)
        info.fields = [ApprovalField(label: "Agente", value: turn.agentName), ApprovalField(label: "Carpeta", value: turn.workspace.path)]
        if let note = description?.trimmingCharacters(in: .whitespacesAndNewlines), !note.isEmpty {
            info.fields.append(ApprovalField(label: "Para qué", value: String(note.prefix(300))))
        }
        if !notes.isEmpty { info.fields.append(ApprovalField(label: "Ojo", value: notes.joined(separator: " "))) }
        // Never log the command itself (it may contain secrets).
        MikaLog.info("Agent command card: \(turn.agentId) (\(command.count) chars)")
        let id = info.requestId
        requestAgent = turn.agentId
        return await withCheckedContinuation { (cont: CheckedContinuation<GateAnswer, Never>) in
            continuation = cont
            requestId = id
            state.pendingApproval = info
            state.isPinned = true
            state.focusId = "agent_\(turn.agentId)"
            SoundEngine.shared.play("approval")
            HookServer.shared.presentApprovalCard()
            Task { @MainActor in
                var waited: TimeInterval = 0
                while self.requestId == id {
                    if turn.cancel.isSet { self.finish(.cancelled, id: id); return }
                    if waited >= Self.cardTimeout { self.finish(.timedOut, id: id); return }
                    try? await Task.sleep(nanoseconds: 250_000_000)
                    waited += 0.25
                }
            }
        }
    }

    /// The card's buttons: true when the click was for an agent's command (and so is handled here).
    /// A card from `requestAgent` is up and no one has answered it.
    func resolve(_ decision: String) -> Bool {
        guard let id = requestId else { return false }
        finish(decision == "allow" || decision == "always" ? .allow : .deny, id: id)
        return true
    }

    private func finish(_ answer: GateAnswer, id: UUID) {
        guard requestId == id, let cont = continuation else { return }
        // The decision is logged (never the command): the answer to "who allowed that?" is in the log.
        MikaLog.info("Agent card decision: \(requestAgent) → \(answer)")
        continuation = nil
        requestId = nil
        let state = AppState.shared
        state.pendingApproval = nil
        state.isPinned = false
        state.view = state.tasks.isEmpty ? .empty : .overview
        cont.resume(returning: answer)
    }
}
