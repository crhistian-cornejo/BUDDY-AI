import Foundation

enum ClaudeCommand {
    /// `gateRelay`: the path of the `mika-gate` relay when this turn may run commands or use connectors. Bash is then an
    /// available tool (only for an agent that can run) but never a pre-approved one: the only thing that lets a command or a
    /// connector's write through is the relay's hook, after a click. `catalog` is what was discovered of the user's
    /// connectors; the agent's read tools are listed by exact name (which is also what shows the server to the model).
    static func arguments(for agent: AgentDefinition, resume: String?, gateRelay: String? = nil, catalog: [ConnectorInfo] = []) -> [String] {
        let connectors = gateRelay == nil ? [] : agent.connectors
        let useConnectors = !connectors.isEmpty
        // `--safe-mode` switches every hook off, the gate's too; `--restricted` ignores the user's own settings, hooks and
        // CLAUDE.md just the same but keeps the `--settings` hook, and confines the file tools to the workspace. Without
        // `--strict-mcp-config` it loads only the account's claude.ai connectors, whose tools stay hidden unless allowed.
        var args = ["-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages",
                    gateRelay == nil ? "--safe-mode" : "--restricted"]
        if !useConnectors { args += ["--strict-mcp-config", "--mcp-config", #"{"mcpServers":{}}"#] }
        let allowed = AgentCapabilities.claudeTools(for: agent.can)
        let readOnlyConnectorTools = Connectors.readTools(for: connectors, in: catalog)
        let list = (allowed + readOnlyConnectorTools).joined(separator: ",")
        let withShell = gateRelay != nil && agent.can.contains(.run)
        let available = (withShell ? allowed + [AgentGate.shellTool] : allowed).joined(separator: ",")
        args += ["--tools", available, "--allowedTools", list,
                 "--permission-mode", agent.access == .edit ? "acceptEdits" : "dontAsk"]
        if let gateRelay { args += ["--settings", AgentGate.claudeSettings(relayPath: gateRelay, connectors: useConnectors)] }
        let model = ModelCatalog.choice(for: agent, provider: .claude)
        args += ["--model", model.id]
        if let effort = model.effort { args += ["--effort", effort] }       // Haiku takes no effort flag
        let prompt = agent.prompt + (useConnectors ? Connectors.note(for: connectors) : "")
        if !prompt.isEmpty { args += ["--append-system-prompt", prompt] }
        if let resume, !resume.isEmpty { args += ["--resume", resume] }
        return args
    }
}

/// One `claude -p` run: prompt on stdin, events out, SIGINT to cancel.
final class ClaudeTurn: @unchecked Sendable {
    private let process: ProviderProcess
    private let cancelledLock = NSLock()
    private var cancelled = false
    /// Open while this turn may run commands; closing it (the turn ends) shuts the gate for its token.
    private let gate: GateTurnGuard?
    private let gateCancel = GateCancel()

    init(executable: URL, agent: AgentDefinition, resume: String?, workspace: URL, environment: [String: String]) {
        var env = ProviderEnvironment.scrubbed(environment)
        let relay = HookServer.gateScriptPath
        let guardForTurn = FileManager.default.isExecutableFile(atPath: relay)
            ? AgentGate.openTurn(agent: agent, workspace: workspace, cancel: gateCancel) : nil
        if let guardForTurn { env["MIKA_GATE_TOKEN"] = guardForTurn.token }
        gate = guardForTurn
        process = ProviderProcess(executable: executable,
                                  arguments: ClaudeCommand.arguments(for: agent, resume: resume, gateRelay: guardForTurn == nil ? nil : relay,
                                                                   catalog: ConnectorStore.standard.load()),
                                  environment: env,
                                  currentDirectory: workspace)
    }

    func cancel() {
        cancelledLock.withLock { cancelled = true }
        gateCancel.set()
        process.interrupt()
    }

    /// Last resort when `cancel` gets no answer.
    func abort() {
        cancelledLock.withLock { cancelled = true }
        gateCancel.set()
        process.terminate()
    }

    private var isCancelled: Bool { cancelledLock.withLock { cancelled } }

    func events(prompt: String) -> AsyncStream<ProviderEvent> {
        AsyncStream { continuation in
            let task = Task {
                // A Stop pressed before the child exists must not let it run.
                if isCancelled { continuation.yield(.done); continuation.finish(); return }
                do { try process.start(stdin: Data(prompt.utf8)) } catch {
                    continuation.yield(.failure(ProviderFailure(kind: .cli, message: "No se pudo iniciar el cliente de Claude.")))
                    continuation.finish()
                    return
                }
                if isCancelled { process.interrupt() }          // cancelled while the child was starting
                var parser = ClaudeStreamParser()
                var finished = false
                for await line in process.lines {
                    for event in parser.feed(line) {
                        switch event { case .done, .failure: finished = true; default: break }
                        continuation.yield(event)
                    }
                }
                _ = await process.waitUntilExit()
                if !finished {
                    if isCancelled {
                        continuation.yield(.done)
                    } else {
                        let detail = process.stderrText
                        continuation.yield(.failure(ProviderFailure(
                            kind: ProviderFailure.classify(detail),
                            message: detail.isEmpty ? "El cliente de Claude no devolvió una respuesta." : String(detail.prefix(400)))))
                    }
                }
                continuation.finish()
            }
            continuation.onTermination = { [process] _ in
                task.cancel()
                process.terminate()
            }
        }
    }
}
