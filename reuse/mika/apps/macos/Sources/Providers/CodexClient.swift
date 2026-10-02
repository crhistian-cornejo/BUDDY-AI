import Foundation

enum CodexEventMapper {
    static func map(method: String, params: JSONValue, turnId: String) -> [ProviderEvent] {
        switch method {
        case "item/agentMessage/delta":
            guard params["turnId"].stringValue == turnId, let delta = params["delta"].stringValue, !delta.isEmpty else { return [] }
            return [.delta(delta)]
        case "item/started":
            guard params["turnId"].stringValue == turnId, let type = params["item"]["type"].stringValue else { return [] }
            if type == "imageGeneration" { return [.imageStarted] }
            if type == "webSearch" { return [.tool(name: type, summary: searchQuery(params["item"]))] }
            return ["commandExecution", "mcpToolCall", "fileChange"].contains(type) ? [.tool(name: type, summary: "")] : []
        case "item/completed":
            guard params["turnId"].stringValue == turnId, let type = params["item"]["type"].stringValue else { return [] }
            if type == "webSearch" {
                let action = params["item"]["action"]
                if action["type"].stringValue == "openPage", let url = action["url"].stringValue,
                   let source = ChatSource.make(title: "", url: url) {
                    return [.source(title: source.title, url: source.url)]
                }
                let query = searchQuery(params["item"])
                return query.isEmpty ? [] : [.tool(name: type, summary: query)]
            }
            guard type == "imageGeneration" else { return [] }
            if let path = params["item"]["savedPath"].stringValue { return [.image(path: path)] }
            let failure = params["item"]["failure"]
            guard failure != .null else { return [] }
            return [.imageFailed(imageFailureMessage(failure))]
        case "turn/completed":
            guard params["turn"]["id"].stringValue == turnId else { return [] }
            if params["turn"]["status"].stringValue == "failed" {
                let message = params["turn"]["error"]["message"].stringValue ?? "Codex no pudo completar la respuesta."
                return [.failure(ProviderFailure(kind: ProviderFailure.classify(message), message: message))]
            }
            return [.done]
        case "error":
            guard params["turnId"].stringValue == turnId else { return [] }
            if params["willRetry"].boolValue == true { return [.progress("Reintentando…")] }
            let message = params["error"]["message"].stringValue ?? "Codex devolvió un error."
            return [.failure(ProviderFailure(kind: ProviderFailure.classify(message), message: message))]
        default:
            return []
        }
    }
}

/// What a web search item is looking for: its own `query`, or the one inside its `action`.
private func searchQuery(_ item: JSONValue) -> String {
    for candidate in [item["query"].stringValue, item["action"]["query"].stringValue] {
        if let q = candidate?.trimmingCharacters(in: .whitespacesAndNewlines), !q.isEmpty { return q }
    }
    return ""
}

/// The only failure the server reports today is `usageLimitExceeded` (with the reset time); anything else is generic.
private func imageFailureMessage(_ failure: JSONValue) -> String {
    guard failure["type"].stringValue == "usageLimitExceeded" else { return "No se pudo crear la imagen." }
    guard let resets = failure["resetsAt"].intValue else { return "Límite de uso alcanzado; no se pudo crear la imagen." }
    let formatter = DateFormatter()
    formatter.locale = Locale(identifier: "es")
    formatter.dateFormat = "EEEE HH:mm"
    return "Límite de uso alcanzado; no se pudo crear la imagen. Se reinicia el \(formatter.string(from: Date(timeIntervalSince1970: TimeInterval(resets))))."
}

enum CodexRequests {
    private static func encode(_ value: JSONValue) -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.withoutEscapingSlashes]
        return String(decoding: (try? encoder.encode(value)) ?? Data(), as: UTF8.self)
    }

    /// `experimental`: the thread registers MIKA's own tools (`dynamicTools`), which Codex only accepts from a client that
    /// said so.
    static func initialize(id: Int, experimental: Bool = false) -> String {
        var params: [String: JSONValue] = ["clientInfo": .object(["name": .string("mika"), "title": .string("MIKA"),
                                                                  "version": .string("0.1.1")])]
        if experimental { params["capabilities"] = .object(["experimentalApi": .bool(true)]) }
        return encode(.object(["id": .number(Double(id)), "method": .string("initialize"), "params": .object(params)]))
    }

    static let initialized = #"{"method":"initialized"}"#

    /// MIKA's own fixed settings for `codex app-server`, as on Windows (codex_server.rs): live web search, ChatGPT login
    /// only, no shell, no MCP servers or apps from the user's Codex configuration. An agent that reads what strangers
    /// post (PARLEY's Telegram channels) must not get a shell or the user's tools.
    static let serverArguments = serverArguments(webSearch: true)

    /// `web_search` follows the agent's Web switch.
    static func serverArguments(webSearch: Bool) -> [String] {
        ["app-server", "-c", "web_search=\"\(webSearch ? "live" : "disabled")\"", "-c", #"forced_login_method="chatgpt""#,
         "-c", "mcp_servers={}", "--disable", "shell_tool", "--disable", "apps"]
    }

    /// How a Codex answer is laid out so the chat renders it like a Claude one (twin of `CODEX_FORMAT` on Windows).
    static let codexFormat = "Formato de tus respuestas (la app las muestra con Markdown): usa párrafos cortos separados por una línea en blanco; títulos con ## o ### solo si la respuesta tiene varias partes; listas con «- » o «1. » (un elemento por línea, nunca viñetas «•» ni listas en una sola línea); **negrita** para lo importante; tablas Markdown con fila de encabezado y separador |---| cuando comparas datos; bloques de código con ``` y el lenguaje; enlaces como [título](url). No uses HTML, ni emojis decorativos, ni líneas de adornos. Esto prevalece sobre cualquier indicación anterior de «texto plano»."

    static func threadStart(id: Int, agent: AgentDefinition, workspace: URL, connectorSpecs: [JSONValue] = []) -> String {
        var params: [String: JSONValue] = [
            "cwd": .string(workspace.path),
            "sandbox": .string(agent.access == .edit ? "workspace-write" : "read-only"),
            "approvalPolicy": .string("never"),
        ]
        let model = ModelCatalog.choice(for: agent, provider: .codex)
        params["model"] = .string(model.id)
        // The user's own config.toml may say another effort: pin it on the thread (and again on every turn).
        params["config"] = .object(["model_reasoning_effort": .string(model.effort ?? "medium")])
        let connectorNote = connectorSpecs.isEmpty ? "" : Connectors.note(for: agent.connectors)
        params["developerInstructions"] = .string((agent.prompt.isEmpty ? Self.codexFormat : agent.prompt + "\n\n" + Self.codexFormat) + connectorNote)
        // MIKA's own tools (read files, run a command after a click): Codex has no shell, so these are all it has.
        let tools = AgentTools.specs(for: AgentTools.codexTools(for: agent)) + connectorSpecs
        if !tools.isEmpty { params["dynamicTools"] = .array(tools) }
        return encode(.object(["id": .number(Double(id)), "method": .string("thread/start"), "params": .object(params)]))
    }

    /// `images` (absolute paths, PARLEY's Telegram screenshots) go with the text as `localImage` items.
    static func turnStart(id: Int, threadId: String, text: String, effort: String, images: [String] = []) -> String {
        let input = JSONValue.object(["type": .string("text"), "text": .string(text), "text_elements": .array([])])
        let pictures = images.map { JSONValue.object(["type": .string("localImage"), "path": .string($0)]) }
        return encode(.object(["id": .number(Double(id)), "method": .string("turn/start"),
                               "params": .object(["threadId": .string(threadId), "input": .array([input] + pictures),
                                                  "effort": .string(effort)])]))
    }

    static func turnInterrupt(id: Int, threadId: String, turnId: String) -> String {
        encode(.object(["id": .number(Double(id)), "method": .string("turn/interrupt"),
                        "params": .object(["threadId": .string(threadId), "turnId": .string(turnId)])]))
    }
}

/// One `codex app-server` process and one thread per agent, kept alive between turns while MIKA runs.
final class CodexConversation: @unchecked Sendable {
    private typealias Note = (String, JSONValue)
    private static let requestTimeout: Double = 30

    private let executable: URL
    private let agent: AgentDefinition
    private let workspace: URL
    private let environment: [String: String]

    private let lock = NSLock()
    private var process: ProviderProcess?
    private var closed = true
    private var threadId: String?
    private var activeTurnId: String?
    private var turnInFlight = false
    private var cancelRequested = false
    private var nextId = 0
    private var pending: [Int: CheckedContinuation<JSONValue, Never>] = [:]
    private var turnSink: AsyncStream<Note>.Continuation?
    /// Open while a turn may run commands; Stop sets its flag.
    private var gateGuard: GateTurnGuard?
    private var gateTurn: GateTurn?
    /// The connector tools this agent was given (ChatGPT's apps, through the bridge).
    private var connectorTools: [CodexAppTool] = []
    private let gateCancel = GateCancel()

    init(executable: URL, agent: AgentDefinition, workspace: URL, environment: [String: String]) {
        self.executable = executable
        self.agent = agent
        self.workspace = workspace
        self.environment = ProviderEnvironment.scrubbed(environment)
    }

    func shutdown() { teardown(ifCurrent: nil)?.terminate() }

    /// Interrupts the running turn. A Stop pressed before the turn id is known is remembered and sent as soon as it is.
    func cancel() {
        gateCancel.set()
        let target: (String, String)? = lock.withLock {
            if let thread = threadId, let turn = activeTurnId { return (thread, turn) }
            if turnInFlight { cancelRequested = true }
            return nil
        }
        if let (thread, turn) = target { sendInterrupt(thread: thread, turn: turn) }
    }

    private func sendInterrupt(thread: String, turn: String) {
        let id = lock.withLock { () -> Int in nextId += 1; return nextId }
        write(CodexRequests.turnInterrupt(id: id, threadId: thread, turnId: turn))
    }

    /// `images` go with the prompt as `localImage` items (Codex reads pictures from the turn only).
    func send(prompt: String, images: [String] = []) -> AsyncStream<ProviderEvent> {
        AsyncStream { continuation in
            // Marked in-flight synchronously, so a cancel() right after send() is not lost.
            lock.withLock { turnInFlight = true; cancelRequested = false }
            let task = Task { await self.runTurn(prompt: prompt, images: images, continuation: continuation) }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    // MARK: - turn

    private func runTurn(prompt: String, images: [String], continuation: AsyncStream<ProviderEvent>.Continuation) async {
        func fail(_ kind: ProviderFailureKind, _ message: String) {
            continuation.yield(.failure(ProviderFailure(kind: kind, message: message)))
            continuation.finish()
        }
        func failure(_ reply: JSONValue, _ fallback: String) {
            let message = reply["error"]["message"].stringValue ?? fallback
            let kind = ProviderFailure.classify(message)
            fail(kind == .other ? .cli : kind, message)
        }

        defer { lock.withLock { turnInFlight = false; cancelRequested = false; gateGuard = nil; gateTurn = nil } }
        gateCancel.reset()
        if let opened = AgentGate.openTurn(agent: agent, workspace: workspace, cancel: gateCancel), let turn = AgentGate.lookup(token: opened.token) {
            lock.withLock { gateGuard = opened; gateTurn = turn }
        }
        guard await ensureStarted() else { return fail(.cli, "No se pudo iniciar el cliente de Codex.") }
        if lock.withLock({ threadId }) == nil, !agent.connectors.isEmpty {
            let all = await CodexConnectorBridge.shared.allTools()
            lock.withLock { connectorTools = CodexConnectors.tools(for: agent.connectors, in: all) }
        }

        if lock.withLock({ threadId }) == nil {
            let specs = CodexConnectors.specs(for: lock.withLock { connectorTools }, display: { self.connectorName(forSlug: $0) })
            let reply = await request { CodexRequests.threadStart(id: $0, agent: self.agent, workspace: self.workspace, connectorSpecs: specs) }
            guard let thread = reply["result"]["thread"]["id"].stringValue else {
                return failure(reply, "Codex no pudo abrir la conversación.")
            }
            lock.withLock { threadId = thread }
            continuation.yield(.session(thread))
        }
        let thread = lock.withLock { threadId } ?? ""

        if lock.withLock({ cancelRequested }) { continuation.yield(.done); continuation.finish(); return }

        let (stream, sink) = AsyncStream<Note>.makeStream()
        lock.withLock { turnSink = sink }
        defer {
            lock.withLock { turnSink = nil; activeTurnId = nil }
            sink.finish()
        }

        let effort = ModelCatalog.choice(for: agent, provider: .codex).effort ?? "medium"
        let started = await request {
            CodexRequests.turnStart(id: $0, threadId: thread, text: prompt, effort: effort, images: images)
        }
        guard let turnId = started["result"]["turn"]["id"].stringValue else {
            return failure(started, "Codex no pudo iniciar la respuesta.")
        }
        lock.withLock { activeTurnId = turnId }
        if lock.withLock({ cancelRequested }) { sendInterrupt(thread: thread, turn: turnId) }

        for await (method, params) in stream {
            // Plan usage is about the account, not the turn (no turnId): learn it and move on.
            if method == "account/rateLimits/updated" { Limits.shared.recordCodex(params: params); continue }
            for event in CodexEventMapper.map(method: method, params: params, turnId: turnId) {
                continuation.yield(event)
                switch event {
                case .done, .failure: continuation.finish(); return
                default: break
                }
            }
        }
        if Task.isCancelled { continuation.finish(); return }
        fail(.cli, "El cliente de Codex se cerró antes de responder.")
    }

    // MARK: - plumbing

    private static func errorReply(_ message: String) -> JSONValue {
        .object(["error": .object(["message": .string(message)])])
    }

    private func write(_ line: String) {
        let p = lock.withLock { process }
        p?.writeLine(line)
    }

    private func resolve(id: Int, with value: JSONValue) {
        let cont = lock.withLock { pending.removeValue(forKey: id) }
        cont?.resume(returning: value)
    }

    private func request(_ build: (Int) -> String) async -> JSONValue {
        let (id, unavailable) = lock.withLock { () -> (Int, Bool) in
            nextId += 1
            return (nextId, closed || process == nil)
        }
        if unavailable { return Self.errorReply("El cliente de Codex no está disponible.") }
        let line = build(id)
        return await withCheckedContinuation { (cont: CheckedContinuation<JSONValue, Never>) in
            lock.withLock { pending[id] = cont }
            write(line)
            Task { [weak self] in
                try? await Task.sleep(for: .seconds(Self.requestTimeout))
                self?.resolve(id: id, with: Self.errorReply("El cliente de Codex no respondió a tiempo."))
            }
        }
    }

    private func ensureStarted() async -> Bool {
        if lock.withLock({ process != nil && !closed }) { return true }
        let p = ProviderProcess(executable: executable, arguments: CodexRequests.serverArguments(webSearch: agent.can.contains(.web)),
                                environment: environment, currentDirectory: workspace)
        lock.withLock { process = p; closed = false; threadId = nil; activeTurnId = nil; nextId = 0 }
        do { try p.start(stdin: nil, keepStdinOpen: true) } catch {
            _ = teardown(ifCurrent: p)
            return false
        }
        Task { [weak self] in
            for await line in p.lines { self?.handle(line) }
            _ = self?.teardown(ifCurrent: p)
        }
        let tools = !AgentTools.codexTools(for: agent).isEmpty || !agent.connectors.isEmpty
        let hello = await request { CodexRequests.initialize(id: $0, experimental: tools) }
        guard hello["result"] != .null else {
            teardown(ifCurrent: p)?.terminate()
            return false
        }
        write(CodexRequests.initialized)
        return true
    }

    private func handle(_ line: String) {
        guard let value = try? JSONDecoder().decode(JSONValue.self, from: Data(line.utf8)) else { return }
        if let id = value["id"].intValue, value["method"] == .null {
            resolve(id: id, with: value)
        } else if let method = value["method"].stringValue, value["id"] == .null {
            lock.withLock { turnSink }?.yield((method, value["params"]))
        } else if value["method"].stringValue == "item/tool/call", let id = value["id"].intValue {
            // One of MIKA's tools (files, a command after a click): answered off this thread, since a command waits for a click.
            let params = value["params"]
            Task { [weak self] in await self?.answerToolCall(id: id, params: params) }
        } else if value["method"].stringValue != nil, let id = value["id"].intValue {
            // A server request (approval, elicitation…) that P1 does not support: refuse it instead of hanging.
            write(#"{"id":\#(id),"error":{"code":-32601,"message":"MIKA no soporta esta petición"}}"#)
        }
    }

    /// The result of a MIKA tool call, back to Codex. Anything the agent is not allowed is refused, not ignored.
    private func answerToolCall(id: Int, params: JSONValue) async {
        let tool = params["tool"].stringValue ?? ""
        let args = params["arguments"]
        let allowed = AgentTools.codexTools(for: agent)
        var text = "Herramienta no disponible."
        var ok = false
        if let connector = CodexConnectors.resolve(tool, in: lock.withLock({ connectorTools })) {
            if let turn = lock.withLock({ gateTurn }) {
                let result = await AgentGate.runCodexConnector(turn: turn, tool: connector, args: args, display: connectorName(forSlug: connector.slug))
                text = result.text; ok = result.ok
            } else { text = "Este turno no puede usar conectores." }
            return sendToolResult(id: id, text: text, ok: ok)
        }
        if allowed.contains(tool) {
            switch tool {
            case let name where OfficeTools.names.contains(name):
                do {
                    let url = try OfficeTools.run(name, args: args, workspace: workspace)
                    text = "Creado: \(OfficeTools.folder)/\(url.lastPathComponent) en tu carpeta de trabajo. El usuario ya lo tiene a la vista; dile cómo se llama."
                    ok = true
                    let agentCopy = agent
                    await MainActor.run { OfficeAnnounce.created(url, by: agentCopy) }
                } catch { text = (error as? OfficeError)?.message ?? "No se pudo crear el archivo." }
            case AgentTools.list, AgentTools.read:
                do {
                    let result = tool == AgentTools.list ? try AgentTools.listFiles(workspace: workspace, args: args)
                                                         : try AgentTools.readFile(workspace: workspace, args: args)
                    text = Self.render(result); ok = true
                } catch { text = (error as? AgentToolError)?.message ?? "No se pudo completar." }
            case AgentTools.run:
                if let turn = lock.withLock({ gateTurn }) {
                    let result = await AgentGate.runCommand(turn: turn, args: args)
                    text = result.text; ok = result.ok
                } else { text = "Este turno no puede ejecutar comandos." }
            default: break
            }
        }
        sendToolResult(id: id, text: text, ok: ok)
    }

    private func sendToolResult(id: Int, text: String, ok: Bool) {
        let item = JSONValue.object(["type": .string("inputText"), "text": .string(text)])
        let reply = JSONValue.object(["id": .number(Double(id)),
                                      "result": .object(["contentItems": .array([item]), "success": .bool(ok)])])
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.withoutEscapingSlashes]
        write(String(decoding: (try? encoder.encode(reply)) ?? Data(), as: UTF8.self))
    }

    /// The connector's name as the user knows it ("Google Calendar") from the bridge's slug ("google_calendar").
    private func connectorName(forSlug slug: String) -> String {
        agent.connectors.first { CodexConnectors.slug(for: $0) == slug } ?? slug.replacingOccurrences(of: "_", with: " ")
    }

    private static func render(_ value: JSONValue) -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.withoutEscapingSlashes]
        return String(decoding: (try? encoder.encode(value)) ?? Data(), as: UTF8.self)
    }

    /// Detaches the process and fails everything that was waiting on it. No-op for a stale process.
    @discardableResult
    private func teardown(ifCurrent expected: ProviderProcess?) -> ProviderProcess? {
        let (p, waiting, sink) = lock.withLock { () -> (ProviderProcess?, [Int: CheckedContinuation<JSONValue, Never>], AsyncStream<Note>.Continuation?) in
            if let expected, process !== expected { return (nil, [:], nil) }
            let p = process
            let w = pending, s = turnSink
            process = nil; closed = true; threadId = nil; activeTurnId = nil; pending = [:]; turnSink = nil
            return (p, w, s)
        }
        waiting.values.forEach { $0.resume(returning: Self.errorReply("El cliente de Codex se cerró.")) }
        sink?.finish()
        return p
    }
}
