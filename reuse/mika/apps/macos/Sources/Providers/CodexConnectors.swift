import Foundation

// The user's ChatGPT connectors (Gmail, Google Calendar, Drive, GitHub, Notion…) for MIKA's Codex agents.
//
// Codex runs a connector's writes without asking (verified: Calendar events were created under every approval setting), and
// no setting hides the tools from the model. So the model never sees them. Instead a second, private `codex app-server`
// process (the bridge) has the apps on and never runs a turn; MIKA gives the agent each tool of the connectors it was given
// as one of its own dynamic tools, and when the agent calls one MIKA decides: a tool that Codex itself marks read-only
// (`readOnlyHint`) is called straight away, anything else opens the approval card first, and a tool with no annotation counts
// as a write. The call is then made by the bridge with `mcpServer/tool/call`.

struct CodexAppTool: Equatable, Sendable {
    /// "google_calendar" — the part of the tool's name before the dot.
    var slug: String
    /// "create_event"
    var name: String
    var description: String
    var inputSchema: JSONValue
    var readOnly: Bool
    var destructive: Bool

    /// What the bridge is asked to call: `google_calendar.create_event`.
    var fullName: String { slug + "." + name }

    /// What the agent calls: `google_calendar__create_event` (letters, digits, `_` and `-`, at most 64).
    var dynamicName: String {
        String((slug + "__" + name).unicodeScalars.map { ($0.properties.isAlphabetic || ($0 >= "0" && $0 <= "9") || $0 == "_" || $0 == "-") ? Character($0) : "_" })
    }
}

enum CodexConnectors {
    static let server = "codex_apps"
    static let maxNameLength = 64

    /// "Google Calendar" → "google_calendar": how the bridge names an app.
    static func slug(for connectorName: String) -> String { Connectors.token(connectorName).lowercased() }

    /// The tools of the `codex_apps` server out of an `mcpServerStatus/list` result.
    static func parseStatus(_ result: JSONValue) -> [CodexAppTool] {
        guard case .array(let servers) = result["data"] else { return [] }
        var out: [CodexAppTool] = []
        for entry in servers where entry["name"].stringValue == server {
            guard case .object(let tools) = entry["tools"] else { continue }
            for (full, tool) in tools {
                guard let dot = full.firstIndex(of: "."), dot != full.startIndex else { continue }
                let slug = String(full[..<dot]), name = String(full[full.index(after: dot)...])
                guard !name.isEmpty else { continue }
                let annotations = tool["annotations"]
                var schema = tool["inputSchema"]
                if schema["type"].stringValue != "object" { schema = .object(["type": .string("object"), "properties": .object([:])]) }
                out.append(CodexAppTool(slug: slug, name: name, description: tool["description"].stringValue ?? "", inputSchema: schema,
                                        // Missing means unknown, and unknown is a write.
                                        readOnly: annotations["readOnlyHint"].boolValue == true,
                                        destructive: annotations["destructiveHint"].boolValue == true))
            }
        }
        return out.sorted { $0.fullName < $1.fullName }
    }

    /// The tools of the connectors an agent has, whose names the model can use.
    static func tools(for connectors: [String], in all: [CodexAppTool]) -> [CodexAppTool] {
        let wanted = Set(connectors.map(slug(for:)))
        return all.filter { wanted.contains($0.slug) && $0.dynamicName.count <= maxNameLength }
    }

    static func resolve(_ dynamicName: String, in tools: [CodexAppTool]) -> CodexAppTool? { tools.first { $0.dynamicName == dynamicName } }

    /// `dynamicTools` entries for `thread/start`.
    static func specs(for tools: [CodexAppTool], display: (String) -> String) -> [JSONValue] {
        tools.map { tool in
            let note = tool.readOnly ? "" : (tool.destructive ? " [Borra o cambia datos: el usuario debe aprobarlo con un clic.]" : " [Cambia datos: el usuario debe aprobarlo con un clic.]")
            let text = "[\(display(tool.slug))] " + String(tool.description.prefix(500)) + note
            return .object(["type": .string("function"), "name": .string(tool.dynamicName), "description": .string(text), "inputSchema": tool.inputSchema])
        }
    }

    /// The text of a tool call's result for the model.
    static func render(_ result: JSONValue) -> (text: String, ok: Bool) {
        var parts: [String] = []
        if case .array(let content) = result["content"] {
            for item in content where item["type"].stringValue == "text" { if let text = item["text"].stringValue { parts.append(text) } }
        }
        // A connector often says "Action completed." in the text and puts the data in `structuredContent`: both go to the model.
        if result["structuredContent"] != .null, result["structuredContent"] != .object([:]) {
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.withoutEscapingSlashes]
            parts.append(String(decoding: (try? encoder.encode(result["structuredContent"])) ?? Data(), as: UTF8.self))
        }
        return (AgentTools.clip(parts.joined(separator: "\n"), AgentTools.maxModelOutput), result["isError"].boolValue != true)
    }
}

/// The private `codex app-server` that owns the apps. Started on first use, stopped after ten idle minutes.
final class CodexConnectorBridge: @unchecked Sendable {
    static let shared = CodexConnectorBridge()

    private let lock = NSLock()
    private var process: ProviderProcess?
    private var startTask: Task<Bool, Never>?
    private var nextId = 0
    private var pending: [Int: CheckedContinuation<JSONValue, Never>] = [:]
    private var threadId: String?
    private var cache: (at: Date, tools: [CodexAppTool])?
    private var idle: Task<Void, Never>?

    private static let toolsTTL: TimeInterval = 600

    // MARK: use

    /// Every tool of the user's accessible apps (cached ten minutes).
    func allTools() async -> [CodexAppTool] {
        if let cache = lock.withLock({ cache }), Date().timeIntervalSince(cache.at) < Self.toolsTTL { touch(); return cache.tools }
        guard await ensure() else { return [] }
        let reply = await request("mcpServerStatus/list", ["detail": .string("full")], timeout: 60)
        let tools = CodexConnectors.parseStatus(reply["result"])
        if !tools.isEmpty { lock.withLock { cache = (Date(), tools) } }
        touch()
        return tools
    }

    /// The names of the apps the account has connected and can use.
    func appNames() async -> [String] {
        guard await ensure() else { return [] }
        let reply = await request("app/list", [:], timeout: 60)
        touch()
        guard case .array(let apps) = reply["result"]["data"] else { return [] }
        return apps.filter { $0["isAccessible"].boolValue == true && $0["isEnabled"].boolValue == true }.compactMap { $0["name"].stringValue }
    }

    /// Calls one tool with the user's own connection. Never call this without the gate.
    func call(_ tool: CodexAppTool, arguments: JSONValue) async -> (text: String, ok: Bool) {
        guard await ensure(), let thread = lock.withLock({ threadId }) else { return ("Los conectores no están disponibles ahora.", false) }
        let reply = await request("mcpServer/tool/call", ["threadId": .string(thread), "server": .string(CodexConnectors.server),
                                                          "tool": .string(tool.fullName), "arguments": arguments], timeout: 90)
        touch()
        if reply["error"] != .null { return (reply["error"]["message"].stringValue ?? "El conector no respondió.", false) }
        return CodexConnectors.render(reply["result"])
    }

    func shutdown() {
        let (p, waiting) = lock.withLock { () -> (ProviderProcess?, [Int: CheckedContinuation<JSONValue, Never>]) in
            let p = process, w = pending
            process = nil; pending = [:]; threadId = nil; startTask = nil; cache = nil
            return (p, w)
        }
        waiting.values.forEach { $0.resume(returning: Self.failure("El puente de conectores se cerró.")) }
        p?.terminate()
    }

    // MARK: process

    private func touch() {
        idle?.cancel()
        idle = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 600_000_000_000)
            if !Task.isCancelled { self?.shutdown() }
        }
    }

    private func ensure() async -> Bool {
        if let task = lock.withLock({ startTask }) { return await task.value }
        let task = Task { await self.startProcess() }
        lock.withLock { startTask = task }
        let ok = await task.value
        if !ok { lock.withLock { startTask = nil } }
        return ok
    }

    private func startProcess() async -> Bool {
        guard let codex = CLILocator.find("codex") else { return false }
        // Apps on (that is the point); no shell, no MCP servers of the user's own, ChatGPT sign-in only.
        let p = ProviderProcess(executable: codex, arguments: ["app-server", "-c", #"forced_login_method="chatgpt""#, "-c", "mcp_servers={}", "--disable", "shell_tool"],
                                environment: ProviderEnvironment.scrubbed(ProcessInfo.processInfo.environment),
                                currentDirectory: FileManager.default.temporaryDirectory)
        lock.withLock { process = p }
        do { try p.start(stdin: nil, keepStdinOpen: true) } catch { shutdown(); return false }
        Task { [weak self] in
            for await line in p.lines { self?.handle(line, from: p) }
            self?.ended(p)
        }
        let hello = await request("initialize", ["clientInfo": .object(["name": .string("mika"), "title": .string("MIKA"), "version": .string("0.1.1")]),
                                                 "capabilities": .object(["experimentalApi": .bool(true)])], timeout: 30)
        guard hello["result"] != .null else { shutdown(); return false }
        p.writeLine(CodexRequests.initialized)
        // A thread only to have an id: no turn is ever started on this process, so no model ever runs here.
        let thread = await request("thread/start", ["cwd": .string(FileManager.default.temporaryDirectory.path), "sandbox": .string("read-only"),
                                                    "approvalPolicy": .string("never")], timeout: 30)
        guard let id = thread["result"]["thread"]["id"].stringValue else { shutdown(); return false }
        lock.withLock { threadId = id }
        return true
    }

    private func handle(_ line: String, from p: ProviderProcess) {
        guard let value = try? JSONDecoder().decode(JSONValue.self, from: Data(line.utf8)) else { return }
        if let id = value["id"].intValue, value["method"] == .null {
            let cont = lock.withLock { pending.removeValue(forKey: id) }
            cont?.resume(returning: value)
        } else if value["method"].stringValue != nil, let id = value["id"].intValue {
            p.writeLine(#"{"id":\#(id),"error":{"code":-32601,"message":"MIKA no soporta esta petición"}}"#)     // a request from the server: refused
        }
    }

    private func ended(_ p: ProviderProcess) {
        let isCurrent = lock.withLock { process === p }
        if isCurrent { shutdown() }
    }

    private func request(_ method: String, _ params: [String: JSONValue], timeout: TimeInterval) async -> JSONValue {
        let (id, p) = lock.withLock { () -> (Int, ProviderProcess?) in nextId += 1; return (nextId, process) }
        guard let p else { return Self.failure("El puente de conectores no está activo.") }
        let body = JSONValue.object(["id": .number(Double(id)), "method": .string(method), "params": .object(params)])
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.withoutEscapingSlashes]
        let line = String(decoding: (try? encoder.encode(body)) ?? Data(), as: UTF8.self)
        return await withCheckedContinuation { (cont: CheckedContinuation<JSONValue, Never>) in
            lock.withLock { pending[id] = cont }
            p.writeLine(line)
            Task { [weak self] in
                try? await Task.sleep(nanoseconds: UInt64(timeout * 1_000_000_000))
                guard let self else { return }
                let late = self.lock.withLock { self.pending.removeValue(forKey: id) }
                late?.resume(returning: Self.failure("El conector no respondió a tiempo."))
            }
        }
    }

    private static func failure(_ message: String) -> JSONValue { .object(["error": .object(["message": .string(message)])]) }
}
