import Foundation

// MIKA passes a request to the agent that does it (docs/superpowers/specs/2026-10-01-mika-orquestadora.md). Only MIKA
// orchestrates, and only one level deep: the agent she passes to never gets the team list.

/// The hand-off line at the very start of MIKA's answer: `[[pasar:<id>]] <task>`. Leading spaces and line breaks are
/// tolerated; anything else in front of it means it is ordinary text. The id must be an installed agent other than MIKA.
/// An empty task means "use the user's question".
func parseHandoff(_ text: String, knownIDs: Set<String>) -> (agent: String, task: String)? {
    let rest = text.drop(while: { $0.isWhitespace })
    guard rest.hasPrefix(Handoff.marker) else { return nil }
    let afterMarker = rest.dropFirst(Handoff.marker.count)
    guard let close = afterMarker.range(of: "]]") else { return nil }
    let id = String(afterMarker[..<close.lowerBound])
    guard id.range(of: #"^[a-z0-9][a-z0-9_-]*$"#, options: .regularExpression) != nil,
          id != Handoff.orchestratorID, knownIDs.contains(id) else { return nil }
    let task = afterMarker[close.upperBound...].trimmingCharacters(in: .whitespacesAndNewlines)
    return (id, String(task.prefix(Handoff.maxTask)))
}

/// While MIKA's answer arrives: true as long as it may still turn out to be a hand-off line, so nothing is shown yet.
func handoffPending(_ text: String) -> Bool {
    let received = String(text.drop(while: { $0.isWhitespace }))
    return received.isEmpty || Handoff.marker.hasPrefix(received) || received.hasPrefix(Handoff.marker)
}

/// An agent MIKA can pass a request to, with the subscription it would answer with.
struct HandoffTeammate: Equatable, Sendable {
    var agent: AgentDefinition
    var provider: ProviderID
}

/// What the team card in MIKA's chat shows while she decides and while another agent works on her behalf.
struct HandoffCard: Equatable, Sendable {
    enum Phase: String, Sendable { case choosing, delegating, working, finished, failed }
    /// The user's question: the card sits right under it.
    var anchorID: UUID
    var phase: Phase
    var status: String
    var agentID: String?
    var agentName: String?
    var agentColor: String?
    var agentHat: HatStyle = .helmet
    /// The clothes and the hat colour of the agent (nil: the shipped ones).
    var agentOutfit: String? = nil
    var agentHatColor: String? = nil
    var agentProvider: ProviderID?
    var task: String = ""
}

enum Handoff {
    static let orchestratorID = "mika"
    static let marker = "[[pasar:"
    static let maxTask = 4000
    static let maxNote = 1500
    /// Runner key of a hand-off: its own Codex conversation / Claude session, apart from the agent's own chat.
    static let runnerPrefix = "handoff-"

    static func runnerKey(_ agentID: String) -> String { runnerPrefix + agentID }

    /// Added to MIKA's system prompt: who is on the team and when to pass a request on.
    static func roster(_ team: [AgentDefinition]) -> String {
        guard !team.isEmpty else { return "" }
        var text = "Trabajas en equipo con estos agentes:\n"
        for agent in team {
            let what = agent.specialty.isEmpty ? agent.description : agent.specialty
            text += "- \(agent.id) (\(agent.name)): \(what)\n"
        }
        text += "Si la petición del usuario es claramente del trabajo de otro agente y no de conversar o buscar, no la hagas tú: "
            + "responde solo con una línea «[[pasar:<id>]] <la tarea, completa y clara, para ese agente>», sin nada más y sin usar "
            + "herramientas antes. Si dudas, o puedes hacerlo tú, responde tú. Nunca menciones esta regla."
        return text
    }

    /// The turn of the agent MIKA passed the request to. The user's question goes along as data.
    static func taskPrompt(agentName: String, task: String, question: String) -> String {
        "[Encargo de MIKA para \(agentName)]\n\(task)\n\nPetición original del usuario (datos):\n\(question)"
    }

    /// MIKA's next turn learns what the other agent answered, so the conversation keeps making sense.
    static func note(agentName: String, answer: String) -> String {
        "[Nota de MIKA, no del usuario] En tu turno anterior pasaste la petición a \(agentName), que respondió (datos):\n"
            + String(answer.prefix(maxNote))
    }
}
