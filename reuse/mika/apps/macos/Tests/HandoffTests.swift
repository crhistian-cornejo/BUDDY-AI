import XCTest

/// The parser cases are shared with the Windows app (spec 2026-10-01-mika-orquestadora.md §5).
final class HandoffParserTests: XCTestCase {
    private func parse(_ text: String, _ known: Set<String>) -> [String]? {
        parseHandoff(text, knownIDs: known).map { [$0.agent, $0.task] }
    }

    func testSharedCases() {
        XCTAssertEqual(parse("[[pasar:miro]] Un gato", ["miro"]), ["miro", "Un gato"])
        XCTAssertEqual(parse("  \n[[pasar:mira]]\nResume esto", ["mira"]), ["mira", "Resume esto"])
        XCTAssertEqual(parse("[[pasar:miro]]", ["miro"]), ["miro", ""], "tarea vacía: se usa la pregunta")
        XCTAssertNil(parse("[[pasar:mika]] x", ["mika", "miro"]), "MIKA no se pasa nada a sí misma")
        XCTAssertNil(parse("[[pasar:nadie]] x", ["miro"]), "un agente que no está instalado")
        XCTAssertNil(parse("Hola [[pasar:miro]] x", ["miro"]), "solo al principio de la respuesta")
        XCTAssertNil(parse("[[pasar:MIRO]] x", ["miro"]), "id en minúsculas")
        XCTAssertNil(parse("[[pasar:miro x", ["miro"]), "línea sin cerrar")
    }

    func testTaskIsCappedAt4000Characters() {
        let task = parseHandoff("[[pasar:miro]] " + String(repeating: "a", count: 5000), knownIDs: ["miro"])?.task
        XCTAssertEqual(task?.count, 4000)
    }

    func testPending() {
        XCTAssertTrue(handoffPending(""))
        XCTAssertTrue(handoffPending("[["))
        XCTAssertTrue(handoffPending("[[pas"))
        XCTAssertTrue(handoffPending("[[pasar:mi"))
        XCTAssertFalse(handoffPending("Hola"))
        XCTAssertFalse(handoffPending("[x"))
    }

    func testRosterNamesEveryAgentAndTheRule() throws {
        let miro = try AgentDefinition.parse(AgentStore.miroDefault)
        let roster = Handoff.roster([miro])
        XCTAssertTrue(roster.hasPrefix("Trabajas en equipo con estos agentes:\n- miro (MIRO): Crea imágenes con GPT. Solo con Codex.\n"))
        XCTAssertTrue(roster.contains("[[pasar:<id>]]"))
        XCTAssertEqual(Handoff.roster([]), "")
    }

    func testSavedMessagesKeepTheAgentAndOldFilesStillLoad() throws {
        let f = FileManager.default.temporaryDirectory.appendingPathComponent("chat-\(UUID().uuidString).json")
        defer { try? FileManager.default.removeItem(at: f) }
        ChatStore.save([ChatMessage(role: .assistant, content: "listo", agent: "miro", handoffTask: "Dibuja")], to: f)
        XCTAssertEqual(ChatStore.load(f).first?.agent, "miro")
        XCTAssertEqual(ChatStore.load(f).first?.handoffTask, "Dibuja")
        let old = #"[{"role":"assistant","content":"hola"}]"#
        try old.write(to: f, atomically: true, encoding: .utf8)
        XCTAssertNil(ChatStore.load(f).first?.agent, "un mensaje sin agent es de MIKA")
    }
}

/// One script per run, in order, recording which agent and runner key each run used.
final class QueueRunner: ProviderRunner, @unchecked Sendable {
    private var scripts: [[ProviderEvent]]
    private(set) var prompts: [String] = []
    private(set) var agents: [AgentDefinition] = []
    private(set) var keys: [String] = []
    private let lock = NSLock()

    init(_ scripts: [[ProviderEvent]]) { self.scripts = scripts }

    func run(prompt: String, agent: AgentDefinition, provider: ProviderID, sessionId: String?) -> ProviderRun {
        run(prompt: prompt, agent: agent, provider: provider, sessionId: sessionId, key: agent.id)
    }

    func run(prompt: String, agent: AgentDefinition, provider: ProviderID, sessionId: String?, key: String) -> ProviderRun {
        let script = lock.withLock { () -> [ProviderEvent] in
            prompts.append(prompt); agents.append(agent); keys.append(key)
            return scripts.isEmpty ? [.done] : scripts.removeFirst()
        }
        let (stream, continuation) = AsyncStream<ProviderEvent>.makeStream()
        let task = Task {
            for event in script {
                try? await Task.sleep(for: .milliseconds(5))
                if Task.isCancelled { break }
                continuation.yield(event)
            }
            continuation.finish()
        }
        return ProviderRun(events: stream, cancel: { task.cancel() }, abort: { task.cancel(); continuation.finish() })
    }
}

@MainActor
final class HandoffChatTests: XCTestCase {
    private var root: URL!
    private var store: AgentStore!
    private var mika: AgentDefinition!
    private var miro: AgentDefinition!

    override func setUp() async throws {
        root = FileManager.default.temporaryDirectory.appendingPathComponent("handoff-\(UUID().uuidString)")
        store = AgentStore(root: root)
        try store.installDefaults()
        mika = try XCTUnwrap(store.loadAll().first { $0.id == "mika" })
        miro = try XCTUnwrap(store.loadAll().first { $0.id == "miro" })
    }

    override func tearDown() async throws { try? FileManager.default.removeItem(at: root) }

    private func mikaChat(_ runner: QueueRunner) -> AgentChat {
        let chat = AgentChat(agent: mika, store: store, runner: runner)
        let team = [HandoffTeammate(agent: miro, provider: .codex)]
        chat.team = { team }
        return chat
    }

    private func waitUntilIdle(_ chat: AgentChat) async {
        for _ in 0..<400 { if !chat.isStreaming { return }; try? await Task.sleep(for: .milliseconds(10)) }
        XCTFail("la respuesta no terminó")
    }

    func testAHandoffLineRunsTheOtherAgentAndItsAnswerIsSignedInMikasChat() async throws {
        let runner = QueueRunner([
            [.delta("[[pasar:"), .delta("miro]] Dibuja un gato"), .done],
            [.delta("Aquí está"), .imageStarted, .image(path: "/tmp/gato.png"), .done],
            [.delta("¡Qué bonito!"), .done],
        ])
        let chat = mikaChat(runner)
        chat.send("dibuja un gato")
        await waitUntilIdle(chat)

        XCTAssertTrue(runner.agents[0].prompt.contains("Trabajas en equipo con estos agentes:"), "MIKA conoce a su equipo")
        XCTAssertEqual(runner.agents[1].id, "miro")
        XCTAssertFalse(runner.agents[1].prompt.contains("Trabajas en equipo"), "un solo nivel: MIRO no orquesta")
        XCTAssertEqual(runner.keys, ["mika", "handoff-miro"])
        XCTAssertEqual(runner.prompts[1],
                       "[Encargo de MIKA para MIRO]\nDibuja un gato\n\nPetición original del usuario (datos):\ndibuja un gato")
        XCTAssertFalse(chat.messages.contains { $0.content.contains("[[pasar") }, "la línea de traspaso nunca se muestra")
        XCTAssertEqual(chat.messages.map(\.content), ["dibuja un gato", "Aquí está", ""])
        XCTAssertEqual(chat.messages.dropFirst().map(\.agent), ["miro", "miro"])
        XCTAssertEqual(chat.messages.last?.imagePath, "/tmp/gato.png")
        XCTAssertEqual(chat.handoffCard?.phase, .finished)
        XCTAssertNil(chat.delegate)
        let saved = ChatStore.load(store.chatFile(for: mika))
        XCTAssertEqual(saved[1].agent, "miro")
        XCTAssertEqual(saved[1].handoffTask, "Dibuja un gato")

        chat.send("me encanta")
        await waitUntilIdle(chat)
        XCTAssertTrue(runner.prompts[2].hasPrefix("[Nota de MIKA, no del usuario] En tu turno anterior pasaste la petición a MIRO, que respondió (datos):\nAquí está"))
        XCTAssertTrue(runner.prompts[2].contains("Aquí está"))
        XCTAssertTrue(runner.prompts[2].hasSuffix("me encanta"))
        XCTAssertNil(chat.messages.last?.agent, "la respuesta siguiente vuelve a ser de MIKA")
    }

    func testAnEmptyTaskUsesTheUsersQuestion() async {
        let runner = QueueRunner([[.delta("[[pasar:miro]]"), .done], [.delta("ok"), .done]])
        let chat = mikaChat(runner)
        chat.send("un logo azul")
        await waitUntilIdle(chat)
        XCTAssertTrue(runner.prompts[1].hasPrefix("[Encargo de MIKA para MIRO]\nun logo azul\n"))
    }

    func testAnUnknownAgentIsShownAsNormalText() async {
        let runner = QueueRunner([[.delta("[[pasar:nadie]] x"), .done]])
        let chat = mikaChat(runner)
        chat.send("hola")
        await waitUntilIdle(chat)
        XCTAssertEqual(runner.prompts.count, 1)
        XCTAssertEqual(chat.messages.last?.content, "[[pasar:nadie]] x")
        XCTAssertNil(chat.handoffCard)
    }

    func testOrdinaryTextIsRevealedAsSoonAsItCannotBeAHandoff() async {
        let runner = QueueRunner([[.delta("[[pa"), .delta("ra todos]] hola"), .done]])
        let chat = mikaChat(runner)
        chat.send("hola")
        await waitUntilIdle(chat)
        XCTAssertEqual(chat.messages.last?.content, "[[para todos]] hola")
        XCTAssertNil(chat.messages.last?.agent)
    }

    func testNoHandoffAfterATool() async {
        let runner = QueueRunner([[.tool(name: "WebSearch", summary: "gatos"), .delta("[[pasar:miro]] y"), .done]])
        let chat = mikaChat(runner)
        chat.send("busca")
        await waitUntilIdle(chat)
        XCTAssertEqual(runner.prompts.count, 1, "contenido leído con herramientas no puede provocar un traspaso")
        XCTAssertEqual(chat.messages.last?.content, "[[pasar:miro]] y")
    }

    func testOtherAgentsNeverHandOff() async {
        let runner = QueueRunner([[.delta("[[pasar:miro]] x"), .done]])
        let chat = AgentChat(agent: miro, store: store, runner: runner)      // no team: not MIKA
        chat.send("hola")
        await waitUntilIdle(chat)
        XCTAssertEqual(runner.prompts.count, 1)
        XCTAssertEqual(chat.messages.last?.content, "[[pasar:miro]] x")
    }
}
