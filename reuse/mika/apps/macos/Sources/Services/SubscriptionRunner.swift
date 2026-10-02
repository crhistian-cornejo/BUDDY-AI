import Foundation

/// Runs agents through the official CLIs the user is logged into. Never touches credentials.
final class SubscriptionRunner: ProviderRunner, @unchecked Sendable {
    private let store: AgentStore
    private let lock = NSLock()
    private var codex: [String: CodexConversation] = [:]
    /// Hash of the instructions each live Codex conversation was started with: Codex reads them once per thread, so a
    /// turn whose prompt changed (instructions, skills or the agent's name) starts a new conversation.
    private var codexPrompt: [String: Int] = [:]

    init(store: AgentStore) { self.store = store }

    func run(prompt: String, agent: AgentDefinition, provider: ProviderID, sessionId: String?) -> ProviderRun {
        run(prompt: prompt, agent: agent, provider: provider, sessionId: sessionId, key: agent.id, images: [])
    }

    func run(prompt: String, agent: AgentDefinition, provider: ProviderID, sessionId: String?, key: String) -> ProviderRun {
        run(prompt: prompt, agent: agent, provider: provider, sessionId: sessionId, key: key, images: [])
    }

    /// `key` names the Codex conversation; the agent's own definition and workspace are used either way. `images` are
    /// absolute paths of pictures that go with the prompt (PARLEY's Telegram screenshots): Codex gets them attached as
    /// `localImage`; Claude finds their paths in the prompt and opens them with its Read tool (they are inside the
    /// agent's workspace, its working folder).
    func run(prompt rawPrompt: String, agent: AgentDefinition, provider: ProviderID, sessionId: String?, key: String,
             images: [String]) -> ProviderRun {
        let prompt = ChatPrompt.withToday(rawPrompt)         // today's date travels with every question
        let workspace = store.workspace(for: agent)
        try? FileManager.default.createDirectory(at: workspace, withIntermediateDirectories: true,
                                                 attributes: [.posixPermissions: 0o700])
        let env = ProcessInfo.processInfo.environment
        switch provider {
        case .claude:
            guard let exe = CLILocator.find("claude") else { return Self.failed(.cli, "Claude no está instalado.") }
            let turn = ClaudeTurn(executable: exe, agent: agent, resume: sessionId, workspace: workspace, environment: env)
            return ProviderRun(events: turn.events(prompt: prompt), cancel: { turn.cancel() }, abort: { turn.abort() })
        case .codex:
            guard let exe = CLILocator.find("codex") else { return Self.failed(.cli, "Codex no está instalado.") }
            let promptHash = agent.prompt.hashValue
            let (convo, replaced): (CodexConversation, CodexConversation?) = lock.withLock {
                if let existing = codex[key], codexPrompt[key] == promptHash { return (existing, nil) }
                let stale = codex.removeValue(forKey: key)
                let created = CodexConversation(executable: exe, agent: agent, workspace: workspace, environment: env)
                codex[key] = created
                codexPrompt[key] = promptHash
                return (created, stale)
            }
            replaced?.shutdown()
            return ProviderRun(events: convo.send(prompt: prompt, images: images), cancel: { convo.cancel() },
                               abort: { convo.shutdown() })
        }
    }

    /// A new chat must not reuse the old Codex thread.
    func reset(agentID: String) {
        let convo = lock.withLock { () -> CodexConversation? in
            codexPrompt.removeValue(forKey: agentID)
            return codex.removeValue(forKey: agentID)
        }
        convo?.shutdown()
    }

    /// "Nuevo" in MIKA's chat: the conversations of every hand-off start over too.
    func resetHandoffs() {
        let convos = lock.withLock { () -> [CodexConversation] in
            let keys = codex.keys.filter { $0.hasPrefix(Handoff.runnerPrefix) }
            keys.forEach { codexPrompt.removeValue(forKey: $0) }
            return keys.compactMap { codex.removeValue(forKey: $0) }
        }
        convos.forEach { $0.shutdown() }
    }

    /// After a conversation is archived, the agent writes down in the background what is worth remembering. Only new
    /// messages count (a reopened conversation is not summarised twice); a failure just leaves the memory as it was.
    func remember(_ conversation: Conversation, agent: AgentDefinition) {
        let new = Array(conversation.messages.dropFirst(min(conversation.memorized, conversation.messages.count)))
        guard new.count >= 2, new.contains(where: { $0.role == .user }) else { return }
        let prompt = ChatArchive.summaryPrompt(new)
        let store = self.store
        Task.detached(priority: .utility) {
            guard let answer = await Self.oneShot(agent: agent, prompt: prompt, store: store) else { return }
            ChatArchive.appendMemory(store, agent.id, notes: ChatArchive.summaryNotes(answer, date: ChatArchive.dateLabel()))
        }
    }

    /// A single answer without a session, for the memory notes. Uses whichever subscription is connected.
    private static func oneShot(agent: AgentDefinition, prompt: String, store: AgentStore) async -> String? {
        var chosen: (ProviderID, URL)?
        for p in [agent.provider] + agent.providers {
            guard await ProviderStatusChecker.check(p).connected, let exe = CLILocator.find(p == .claude ? "claude" : "codex") else { continue }
            chosen = (p, exe)
            break
        }
        guard let chosen else { return nil }
        let (provider, exe) = chosen
        let model = ModelCatalog.choice(for: agent, provider: provider)
        let workspace = store.workspace(for: agent)
        try? FileManager.default.createDirectory(at: workspace, withIntermediateDirectories: true,
                                                 attributes: [.posixPermissions: 0o700])
        let args = provider == .codex
            ? ["exec", "--ignore-user-config", "--skip-git-repo-check", "--sandbox", "read-only", "--disable", "shell_tool",
               "-c", "forced_login_method=\"chatgpt\"", "-c", "model_reasoning_effort=\"low\"", "-m", model.id,
               "--json", "--color", "never", "-"]
            : ["-p", "--output-format", "text", "--safe-mode", "--strict-mcp-config", "--mcp-config", #"{"mcpServers":{}}"#,
               "--setting-sources", "", "--tools", "Read", "--allowedTools", "Read", "--permission-mode", "dontAsk",
               "--model", model.id, "--effort", "low"]
        let process = ProviderProcess(executable: exe, arguments: args,
                                      environment: ProviderEnvironment.scrubbed(ProcessInfo.processInfo.environment),
                                      currentDirectory: workspace)
        do { try process.start(stdin: Data(prompt.utf8)) } catch { return nil }
        let deadline = Task {
            do { try await Task.sleep(for: .seconds(120)); process.terminate() } catch {}
        }
        var lines: [String] = []
        for await line in process.lines { lines.append(line) }
        let code = await process.waitUntilExit()
        deadline.cancel()
        guard code == 0 else { return nil }
        return provider == .claude ? lines.joined(separator: "\n") : ChatArchive.codexExecText(lines)
    }

    /// Runner keys of the turns MIKA starts on her own: `auto-<agent id>`.
    static let automaticPrefix = "auto-"

    /// A turn MIKA runs on her own (PARLEY's reviews), twin of `automatic` in subscription.rs: the agent's prompt and
    /// memory on the first subscription that is connected (its own provider first), in a session of its own
    /// (`auto-<id>`) that is thrown away afterwards, so each review stands on its own. Ends after `limit`. Returns the
    /// whole answer; the caller saves the exchange in the agent's chat. When the primary model has no credits left and
    /// answered nothing, the review is run once more on the agent's fallback model (`ModelCatalog.fallback`), if that
    /// subscription is connected; any other failure is thrown as it always was.
    func automatic(prompt: String, agent: AgentDefinition, images: [String],
                   limit: Duration = .seconds(600), session: String? = nil,
                   onEvent: (@Sendable (ProviderEvent) -> Void)? = nil) async throws -> AutomaticAnswer {
        var chosen: ProviderID?
        for candidate in [agent.provider] + agent.providers {
            if await ProviderStatusChecker.check(candidate).connected { chosen = candidate; break }
        }
        guard let provider = chosen else { throw MessageError("Conecta tu suscripción de Claude o Codex en Ajustes.") }
        var definition = agent
        // Instructions, identity line and skills like any chat turn, then the memory.
        definition.prompt = store.systemPrompt(for: agent) + ChatArchive.memoryPrompt(store, agent.id, query: prompt)
        // `session` gives a long job (the daily digest) a conversation of its own, so it never shares one with a review.
        let key = Self.automaticPrefix + (session.map { $0 + "-" } ?? "") + agent.id
        let fallbackKey = CreditFallback.keyPrefix + key
        defer { reset(agentID: key); reset(agentID: fallbackKey) }       // each review stands on its own
        let started = ContinuousClock.now
        var attempt = await collect(run(prompt: prompt, agent: definition, provider: provider, sessionId: nil, key: key,
                                        images: images), limit: limit, onEvent: onEvent)
        let primary = ModelCatalog.choice(for: definition, provider: provider)
        let fallback = ModelCatalog.fallback(for: agent.id)
        if attempt.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, let failure = attempt.failure,
           failure.isNoCredits, primary != fallback, started.duration(to: .now) < limit,
           await ProviderStatusChecker.check(fallback.provider).connected {
            var retry = definition
            retry.modelOverride = fallback
            attempt = await collect(run(prompt: prompt, agent: retry, provider: fallback.provider, sessionId: nil,
                                        key: fallbackKey, images: images), limit: limit - started.duration(to: .now), onEvent: onEvent)
        }
        if started.duration(to: .now) >= limit {
            throw MessageError("La respuesta excedió el tiempo máximo. Intenta de nuevo.")
        }
        guard !attempt.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            if let failure = attempt.failure { throw failure }
            throw MessageError("El cliente no devolvió una respuesta.")
        }
        return AutomaticAnswer(text: attempt.text, sources: attempt.sources)
    }

    /// What one automatic attempt produced.
    private struct Collected {
        var text = ""
        var sources: [ChatSource] = []
        var failure: ProviderFailure?
    }

    /// Reads one run to its end; past `limit` it is cancelled and, three seconds later, aborted.
    /// `onEvent` sees every event as it comes (what the agent is doing, for the island while a long turn runs).
    private func collect(_ turn: ProviderRun, limit: Duration, onEvent: (@Sendable (ProviderEvent) -> Void)? = nil) async -> Collected {
        let watchdog = Task {
            do { try await Task.sleep(for: limit) } catch { return }
            turn.cancel()
            do { try await Task.sleep(for: .seconds(3)) } catch { return }
            turn.abort()
        }
        defer { watchdog.cancel() }
        var result = Collected()
        for await event in turn.events {
            onEvent?(event)
            switch event {
            case .delta(let chunk):
                result.text += chunk
            case .source(let title, let url):
                if let source = ChatSource.make(title: title, url: url), result.sources.count < 12,
                   !result.sources.contains(where: { $0.url == source.url }) { result.sources.append(source) }
            case .failure(let error):
                result.failure = error
            default:
                break
            }
        }
        return result
    }

    /// Called when MIKA quits: no `codex`/`claude` child may outlive the app.
    func shutdown() {
        let all = lock.withLock { () -> [CodexConversation] in let a = Array(codex.values); codex = [:]; return a }
        all.forEach { $0.shutdown() }
        ProviderProcess.terminateAll()
    }

    private static func failed(_ kind: ProviderFailureKind, _ message: String) -> ProviderRun {
        let (stream, continuation) = AsyncStream<ProviderEvent>.makeStream()
        continuation.yield(.failure(ProviderFailure(kind: kind, message: message)))
        continuation.finish()
        return ProviderRun(events: stream, cancel: {})
    }
}

/// What a turn MIKA ran on her own answered.
struct AutomaticAnswer: Equatable, Sendable {
    var text: String
    var sources: [ChatSource]
}
