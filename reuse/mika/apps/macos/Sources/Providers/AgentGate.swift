import Foundation

// The gate between an agent of MIKA and a command: every command an agent wants to run is put in front of the user on the
// island (the same approval card Claude Code permission requests use) and runs only after an explicit click on Allow. A
// denial, a timeout, a stopped turn, a card that could not be shown or anything unexpected is a refusal: nothing here ever
// allows by itself. Twin of apps/windows/src-tauri/src/services/agent_gate.rs.
//
// Two ways in, one gate:
//  * Claude agents: the turn is started with a PreToolUse hook on the Bash tool (`AgentGate.claudeSettings`) that runs the
//    `mika-gate` relay. The relay forwards the hook to MIKA's socket with the turn's token (the `MIKA_GATE_TOKEN`
//    environment variable of that `claude` process); `judge` asks the user and answers allow or deny. The relay prints a
//    denial whenever it gets no answer at all, and Bash is not in `--allowedTools`, so a hook that fails to run leaves
//    the permission mode (`dontAsk`) to refuse.
//  * Codex agents: the shell is off; `run_command` is a MIKA tool (`item/tool/call`). It asks the user and, only on Allow,
//    runs the command itself in the agent's workspace.

/// A Stop flag shared between the turn and the card that waits for a click.
final class GateCancel: @unchecked Sendable {
    private let lock = NSLock()
    private var flag = false
    var isSet: Bool { lock.withLock { flag } }
    func set() { lock.withLock { flag = true } }
    func reset() { lock.withLock { flag = false } }
}

/// A running turn that may run commands.
struct GateTurn: Sendable {
    var agentId: String
    var agentName: String
    var workspace: URL
    var cancel: GateCancel
    /// Whether the agent may run commands at all.
    var canRun = true
    /// The tokens of the claude.ai connectors the agent has (`Connectors.token`).
    var connectors: Set<String> = []
}

enum GateAnswer: Equatable, Sendable { case allow, deny, timedOut, cancelled, notShown }

enum GateVerdict: Equatable, Sendable {
    case allowed
    case denied(String)
    var isAllowed: Bool { self == .allowed }
}

/// What shows the card and waits for the click. The island in the app; scripted answers in tests.
protocol GatePresenter: Sendable {
    func ask(turn: GateTurn, tool: String, command: String, description: String?, notes: [String]) async -> GateAnswer
}

/// A turn's token while it lives; dropping it closes the gate for that turn.
final class GateTurnGuard: @unchecked Sendable {
    let token: String
    init(token: String) { self.token = token }
    deinit { AgentGate.close(token: token) }
}

/// One card at a time (the island shows one); the others wait their turn.
private actor GateSerial {
    private var busy = false
    func tryAcquire() -> Bool { if busy { return false }; busy = true; return true }
    func release() { busy = false }
}

enum AgentGate {
    private static let lock = NSLock()
    nonisolated(unsafe) private static var turns: [String: GateTurn] = [:]
    /// Tests put a scripted presenter here; nil means the island's.
    nonisolated(unsafe) static var presenter: GatePresenter? = nil
    private static let serial = GateSerial()

    /// The Claude tool the gate covers.
    static let shellTool = "Bash"

    // MARK: Turns

    /// 128 bits from the system's random source: the token cannot be guessed from the clock or a counter.
    static func newToken() -> String {
        var generator = SystemRandomNumberGenerator()
        return (0..<4).map { _ in String(format: "%016llx", generator.next() as UInt64) }.joined()
    }

    /// Opens the gate for one turn of `agent`; nil when the agent can neither run commands nor use connectors.
    static func openTurn(agent: AgentDefinition, workspace: URL, cancel: GateCancel) -> GateTurnGuard? {
        let canRun = agent.can.contains(.run)
            && AgentCapabilities.forbidden(id: agent.id, integration: agent.integration).isDisjoint(with: [.run])
        let connectors = AgentCapabilities.forbidsConnectors(id: agent.id, integration: agent.integration) ? [] : agent.connectors
        guard canRun || !connectors.isEmpty else { return nil }
        let token = newToken()
        lock.withLock {
            turns[token] = GateTurn(agentId: agent.id, agentName: agent.name, workspace: workspace, cancel: cancel,
                                    canRun: canRun, connectors: Connectors.tokens(connectors))
        }
        return GateTurnGuard(token: token)
    }

    static func close(token: String) { lock.withLock { turns[token] = nil } }

    static func lookup(token: String) -> GateTurn? {
        guard token.count >= 16 else { return nil }
        return lock.withLock { turns[token] }
    }

    // MARK: Asking

    /// Asks the user whether `command` may run. Up to three tries while the island cannot show the card (another card is
    /// up); any other answer ends it.
    static func ask(turn: GateTurn, tool: String, command: String, description: String?, notes: [String]? = nil) async -> GateVerdict {
        if command.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return .denied("El comando está vacío.") }
        if command.utf8.count > AgentTools.maxCommand {
            return .denied("El comando es demasiado largo para revisarlo en la isla: no se ejecutó.")
        }
        let stopped = GateVerdict.denied("Respuesta detenida: no se ejecutó.")
        // Waiting for the turn is also interrupted by Stop.
        while true {
            if turn.cancel.isSet { return stopped }
            if await serial.tryAcquire() { break }
            try? await Task.sleep(nanoseconds: 200_000_000)
        }
        let notes = notes ?? AgentTools.commandNotes(command, workspace: turn.workspace)
        for attempt in 0..<3 {
            if turn.cancel.isSet { await serial.release(); return stopped }
            let shower: GatePresenter
            if let custom = presenter { shower = custom } else { shower = await MainActor.run { IslandGatePresenter.shared } }
            switch await shower.ask(turn: turn, tool: tool, command: command, description: description, notes: notes) {
            case .allow: await serial.release(); return .allowed
            case .deny: await serial.release(); return .denied("El usuario denegó el comando: no se ejecutó.")
            case .timedOut: await serial.release(); return .denied("El usuario no respondió a tiempo: no se ejecutó.")
            case .cancelled: await serial.release(); return stopped
            case .notShown: if attempt < 2 { try? await Task.sleep(nanoseconds: 1_500_000_000) }
            }
        }
        await serial.release()
        return .denied("MIKA no pudo mostrar la solicitud en la isla: no se ejecutó.")
    }

    // MARK: Claude (the relay's request)

    /// What a PreToolUse payload asks for: a shell command, or a tool of one of the user's connectors.
    enum HookRequest {
        case command(tool: String, command: String, description: String?)
        case connector(server: String, tool: String, input: [String: Any])
    }

    /// Only the shell tool and the connectors' tools pass, and a call the relay had to cut is refused (the card could not
    /// show all of it).
    static func hookRequest(_ payload: [String: Any]) -> Result<HookRequest, GateRefusal> {
        guard payload["hook_event_name"] as? String == "PreToolUse" else { return .failure(GateRefusal("Solo se atienden solicitudes de herramientas.")) }
        guard let tool = payload["tool_name"] as? String else { return .failure(GateRefusal("Esta puerta solo acepta comandos y conectores.")) }
        if let truncated = payload["_truncated"], (truncated as? Bool) != false {
            return .failure(GateRefusal("La solicitud es demasiado larga para revisarla en la isla: no se ejecutó."))
        }
        let input = payload["tool_input"] as? [String: Any]
        if tool == shellTool {
            guard let command = input?["command"] as? String else { return .failure(GateRefusal("La solicitud no trae un comando.")) }
            return .success(.command(tool: tool, command: command, description: input?["description"] as? String))
        }
        if let split = Connectors.split(tool) { return .success(.connector(server: split.server, tool: tool, input: input ?? [:])) }
        return .failure(GateRefusal("Esta puerta solo acepta comandos y conectores."))
    }

    /// The old name, for the shell command alone.
    static func hookCommand(_ payload: [String: Any]) -> Result<(tool: String, command: String, description: String?), GateRefusal> {
        switch hookRequest(payload) {
        case .success(.command(let tool, let command, let description)): return .success((tool, command, description))
        case .success: return .failure(GateRefusal("Esta puerta solo acepta comandos."))
        case .failure(let why): return .failure(why)
        }
    }

    struct GateRefusal: Error, Equatable { var message: String; init(_ message: String) { self.message = message } }

    /// The answer to a relay request: allow only for a live token and, for anything that changes something, an explicit click.
    static func judge(payload: [String: Any]) async -> GateVerdict {
        guard let token = payload["_gate"] as? String, let turn = lookup(token: token) else {
            return .denied("Este turno no puede ejecutar comandos.")
        }
        switch hookRequest(payload) {
        case .failure(let why): return .denied(why.message)
        case .success(.command(let tool, let command, let description)):
            guard turn.canRun else { return .denied("Este agente no puede ejecutar comandos.") }
            return await ask(turn: turn, tool: tool, command: command, description: description)
        case .success(.connector(let server, let tool, let input)):
            // A server the agent was not given is refused whatever the tool is.
            guard turn.connectors.contains(server) else { return .denied("Este agente no tiene ese conector.") }
            if Connectors.isRead(tool) { return .allowed }
            let name = server.replacingOccurrences(of: "_", with: " ")
            let action = Connectors.split(tool)?.name ?? tool
            return await ask(turn: turn, tool: "\(name): \(action)", command: Connectors.pretty(input), description: nil,
                             notes: ["Cambia datos en tu cuenta de \(name)."])
        }
    }

    /// The line MIKA writes back to the relay.
    static func replyLine(_ verdict: GateVerdict) -> String {
        let body: [String: String]
        switch verdict {
        case .allowed: body = ["permissionDecision": "allow"]
        case .denied(let why): body = ["permissionDecision": "deny", "reason": why]
        }
        let data = (try? JSONSerialization.data(withJSONObject: body)) ?? Data(#"{"permissionDecision":"deny"}"#.utf8)
        return String(decoding: data, as: UTF8.self)
    }

    /// `--settings` for a Claude turn that may run commands: a PreToolUse hook on Bash that runs the relay.
    static func claudeSettings(relayPath: String, connectors: Bool = false) -> String {
        let command = "\"" + relayPath.replacingOccurrences(of: "\"", with: "\\\"") + "\""
        let hook: [String: Any] = ["matcher": connectors ? "\(shellTool)|\(Connectors.toolPrefix).*" : shellTool, "hooks": [["type": "command", "command": command, "timeout": 125]]]
        let data = (try? JSONSerialization.data(withJSONObject: ["hooks": ["PreToolUse": [hook]]], options: [.sortedKeys])) ?? Data("{}".utf8)
        return String(decoding: data, as: UTF8.self)
    }

    // MARK: Codex connectors (the bridge)

    /// A connector tool the agent called through MIKA: a tool Codex marks read-only goes straight to the bridge, anything else
    /// opens the card first. `display` is the connector's name as the user knows it.
    static func runCodexConnector(turn: GateTurn, tool: CodexAppTool, args: JSONValue, display: String) async -> (text: String, ok: Bool) {
        guard turn.connectors.contains(where: { $0.lowercased() == tool.slug }) else { return ("Este agente no tiene ese conector.", false) }
        if !tool.readOnly {
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
            let shown = String(String(decoding: (try? encoder.encode(args)) ?? Data(), as: UTF8.self).prefix(AgentTools.maxCommand))
            let notes = tool.destructive ? ["Puede borrar o cambiar datos de forma difícil de deshacer."] : ["Cambia datos en tu cuenta de \(display)."]
            if case .denied(let why) = await ask(turn: turn, tool: "\(display): \(tool.name)", command: shown, description: nil, notes: notes) {
                return (why + " No lo intentes de otra forma.", false)
            }
        }
        return await CodexConnectorBridge.shared.call(tool, arguments: args)
    }

    // MARK: Codex (the run_command tool)

    /// The `run_command` tool: ask, then (only on Allow) run. The text for the model, whether the call counts as a
    /// success, and the run to show in the chat (nil when nothing ran).
    static func runCommand(turn: GateTurn, args: JSONValue) async -> (text: String, ok: Bool, run: AgentTools.CommandRun?) {
        guard let command = args["command"].stringValue else { return ("Falta el comando.", false, nil) }
        let seconds = min(max(args["timeoutSeconds"].doubleValue ?? AgentTools.defaultTimeout, 1), AgentTools.maxTimeout)
        switch await ask(turn: turn, tool: "Comando", command: command, description: args["description"].stringValue) {
        case .denied(let why): return (why + " No lo intentes de otra forma.", false, nil)
        case .allowed:
            let cancel = turn.cancel
            let run = await AgentTools.execute(command: command, workspace: turn.workspace, timeout: seconds, cancelled: { cancel.isSet })
            let code = run.code.map(String.init) ?? "ninguno (detenido o tiempo agotado)"
            let forModel = "Código de salida: \(code)\n" + AgentTools.clip(run.output, AgentTools.maxModelOutput)
            var shown = run
            shown.output = AgentTools.clip(run.output, AgentTools.maxChatOutput)
            return (forModel, run.code == 0, shown)
        }
    }
}
