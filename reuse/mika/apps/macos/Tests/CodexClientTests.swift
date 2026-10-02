import XCTest

final class CodexClientTests: XCTestCase {
    private func json(_ s: String) -> JSONValue { try! JSONDecoder().decode(JSONValue.self, from: Data(s.utf8)) }

    private func agent() -> AgentDefinition {
        AgentDefinition(id: "x", name: "X", color: "#fff", description: "", providers: [.codex], provider: .codex, model: "smart",
                        access: .read, accepts: [], tools: [], integration: nil, prompt: "Eres X.")
    }

    // MARK: mapping

    func testDeltaOfThisTurnIsMappedAndOtherTurnsAreIgnored() {
        XCTAssertEqual(CodexEventMapper.map(method: "item/agentMessage/delta",
                                            params: json(#"{"delta":"Ho","itemId":"i","threadId":"t","turnId":"T1"}"#), turnId: "T1"), [.delta("Ho")])
        XCTAssertEqual(CodexEventMapper.map(method: "item/agentMessage/delta",
                                            params: json(#"{"delta":"x","itemId":"i","threadId":"t","turnId":"OTRO"}"#), turnId: "T1"), [])
    }

    func testTurnCompletedStatuses() {
        func completed(_ status: String, error: String? = nil) -> JSONValue {
            let e = error.map { #","error":{"message":"\#($0)"}"# } ?? ""
            return json(#"{"threadId":"t","turn":{"id":"T1","status":"\#(status)","items":[]\#(e)}}"#)
        }
        XCTAssertEqual(CodexEventMapper.map(method: "turn/completed", params: completed("completed"), turnId: "T1"), [.done])
        XCTAssertEqual(CodexEventMapper.map(method: "turn/completed", params: completed("interrupted"), turnId: "T1"), [.done])
        let failed = CodexEventMapper.map(method: "turn/completed", params: completed("failed", error: "Usage limit reached, try again later"), turnId: "T1")
        guard case .failure(let f)? = failed.first else { return XCTFail("expected failure") }
        XCTAssertEqual(f.kind, .limit)
    }

    func testErrorNotificationRetryVersusFinal() {
        let retry = CodexEventMapper.map(method: "error", params: json(#"{"error":{"message":"net"},"threadId":"t","turnId":"T1","willRetry":true}"#), turnId: "T1")
        XCTAssertEqual(retry, [.progress("Reintentando…")])
        let final = CodexEventMapper.map(method: "error", params: json(#"{"error":{"message":"Please sign in"},"threadId":"t","turnId":"T1","willRetry":false}"#), turnId: "T1")
        guard case .failure(let f)? = final.first else { return XCTFail("expected failure") }
        XCTAssertEqual(f.kind, .auth)
    }

    func testToolAndImageItems() {
        XCTAssertEqual(CodexEventMapper.map(method: "item/started", params: json(#"{"threadId":"t","turnId":"T1","item":{"type":"webSearch","id":"w"}}"#), turnId: "T1"),
                       [.tool(name: "webSearch", summary: "")])
        XCTAssertEqual(CodexEventMapper.map(method: "item/completed", params: json(#"{"threadId":"t","turnId":"T1","item":{"type":"imageGeneration","id":"g","savedPath":"/tmp/a.png"}}"#), turnId: "T1"),
                       [.image(path: "/tmp/a.png")])
        XCTAssertEqual(CodexEventMapper.map(method: "item/started", params: json(#"{"threadId":"t","turnId":"T1","item":{"type":"reasoning","id":"r"}}"#), turnId: "T1"), [])
    }

    func testImageGenerationStartIsTheSignalForTheSkeleton() {
        let started = #"{"threadId":"t","turnId":"T1","item":{"type":"imageGeneration","id":"g","status":"in_progress","result":""}}"#
        XCTAssertEqual(CodexEventMapper.map(method: "item/started", params: json(started), turnId: "T1"), [.imageStarted])
        XCTAssertEqual(CodexEventMapper.map(method: "item/started", params: json(started), turnId: "OTRO"), [])
    }

    func testImageGenerationFailureBecomesAReadableMessage() {
        let limit = #"{"threadId":"t","turnId":"T1","item":{"type":"imageGeneration","id":"g","status":"failed","result":"","failure":{"type":"usageLimitExceeded","limitId":"codex","resetsAt":1791164335}}}"#
        guard case .imageFailed(let message)? = CodexEventMapper.map(method: "item/completed", params: json(limit), turnId: "T1").first
        else { return XCTFail("expected imageFailed") }
        XCTAssertTrue(message.lowercased().contains("límite"), message)
        XCTAssertTrue(message.lowercased().contains("reinicia"), message)

        let other = #"{"threadId":"t","turnId":"T1","item":{"type":"imageGeneration","id":"g","status":"failed","result":"","failure":"boom"}}"#
        XCTAssertEqual(CodexEventMapper.map(method: "item/completed", params: json(other), turnId: "T1"),
                       [.imageFailed("No se pudo crear la imagen.")])

        let neither = #"{"threadId":"t","turnId":"T1","item":{"type":"imageGeneration","id":"g","status":"completed","result":""}}"#
        XCTAssertEqual(CodexEventMapper.map(method: "item/completed", params: json(neither), turnId: "T1"), [],
                       "sin ruta ni fallo no hay nada que mostrar")
    }

    func testRequestShapes() throws {
        let start = json(CodexRequests.threadStart(id: 2, agent: agent(), workspace: URL(fileURLWithPath: "/tmp/ws")))
        XCTAssertEqual(start["method"].stringValue, "thread/start")
        XCTAssertEqual(start["params"]["sandbox"].stringValue, "read-only")
        XCTAssertEqual(start["params"]["approvalPolicy"].stringValue, "never")
        XCTAssertEqual(start["params"]["cwd"].stringValue, "/tmp/ws")
        XCTAssertEqual(start["params"]["developerInstructions"].stringValue, "Eres X.\n\n" + CodexRequests.codexFormat)
        XCTAssertEqual(start["params"]["model"].stringValue, "gpt-6.1-sol")
        XCTAssertEqual(start["params"]["config"]["model_reasoning_effort"].stringValue, "medium", "el esfuerzo se fija en el hilo, no solo por turno")
        XCTAssertTrue(CodexRequests.threadStart(id: 2, agent: agent(), workspace: URL(fileURLWithPath: "/tmp/ws")).contains("\"thread/start\""),
                      "las barras no deben escaparse")
        let turn = json(CodexRequests.turnStart(id: 3, threadId: "th", text: "hola\n\"q\"", effort: "medium"))
        XCTAssertEqual(turn["params"]["threadId"].stringValue, "th")
        XCTAssertEqual(turn["params"]["effort"].stringValue, "medium")
        XCTAssertEqual(turn["params"]["input"], .array([.object(["type": .string("text"), "text": .string("hola\n\"q\""), "text_elements": .array([])])]))
        XCTAssertEqual(json(CodexRequests.turnInterrupt(id: 4, threadId: "th", turnId: "T"))["params"]["turnId"].stringValue, "T")
        var edit = agent(); edit.access = .edit
        XCTAssertEqual(json(CodexRequests.threadStart(id: 5, agent: edit, workspace: URL(fileURLWithPath: "/tmp/ws")))["params"]["sandbox"].stringValue, "workspace-write")
    }

    /// Same fixed settings as codex_server.rs on Windows: no shell, no MCP servers or apps from the user's config.
    func testTheServerStartsWithMikasOwnSettings() {
        XCTAssertEqual(CodexRequests.serverArguments, ["app-server", "-c", "web_search=\"live\"", "-c",
                                                       "forced_login_method=\"chatgpt\"", "-c", "mcp_servers={}",
                                                       "--disable", "shell_tool", "--disable", "apps"])
    }

    /// PARLEY's Telegram screenshots go with the text as `localImage` items (codex_server.rs `start_turn`).
    func testTurnImagesGoAsLocalImages() {
        let turn = json(CodexRequests.turnStart(id: 6, threadId: "th", text: "mira", effort: "medium",
                                                images: ["/ws/telegram/media/100_1.jpg", "/ws/telegram/media/100_2.jpg"]))
        XCTAssertEqual(turn["params"]["input"], .array([
            .object(["type": .string("text"), "text": .string("mira"), "text_elements": .array([])]),
            .object(["type": .string("localImage"), "path": .string("/ws/telegram/media/100_1.jpg")]),
            .object(["type": .string("localImage"), "path": .string("/ws/telegram/media/100_2.jpg")]),
        ]))
    }

    // MARK: conversation against a fake `codex app-server`

    /// Speaks just enough JSON-RPC; it echoes each request id like the real server does.
    /// `holdTurn` keeps the turn open until it receives `turn/interrupt`; `delayStart` delays the `turn/start` reply.
    private func fakeServer(deltas: [String], failOnTurn: Bool = false, holdTurn: Bool = false, delayStart: Double = 0) throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("fake-codex-\(UUID().uuidString).sh")
        let emit = deltas.map {
            #"printf '%s\n' '{"method":"item/agentMessage/delta","params":{"delta":"\#($0)","itemId":"i","threadId":"th1","turnId":"T1"}}'"#
        }.joined(separator: "\n      ")
        let tail: String
        if holdTurn { tail = ":" }
        else if failOnTurn { tail = #"printf '%s\n' '{"method":"turn/completed","params":{"threadId":"th1","turn":{"id":"T1","status":"failed","items":[],"error":{"message":"Not logged in"}}}}'"# }
        else { tail = #"printf '%s\n' '{"method":"turn/completed","params":{"threadId":"th1","turn":{"id":"T1","status":"completed","items":[]}}}'"# }
        let script = #"""
        #!/bin/sh
        while IFS= read -r line; do
          id=$(printf '%s' "$line" | sed -E 's/.*"id":([0-9]+).*/\1/')
          case "$line" in
            *'"method":"initialize"'*) printf '{"id":%s,"result":{"userAgent":"fake"}}\n' "$id" ;;
            *'"method":"thread/start"'*) printf '{"id":%s,"result":{"thread":{"id":"th1"},"model":"m"}}\n' "$id" ;;
            *'"method":"turn/start"'*)
              sleep \#(delayStart)
              printf '{"id":%s,"result":{"turn":{"id":"T1","status":"inProgress","items":[]}}}\n' "$id"
              \#(emit)
              \#(tail) ;;
            *'"method":"turn/interrupt"'*)
              printf '{"id":%s,"result":{}}\n' "$id"
              printf '%s\n' '{"method":"turn/completed","params":{"threadId":"th1","turn":{"id":"T1","status":"interrupted","items":[]}}}' ;;
          esac
        done
        """#
        try script.write(to: url, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: url.path)
        return url
    }

    private func conversation(_ exe: URL) -> CodexConversation {
        CodexConversation(executable: exe, agent: agent(), workspace: FileManager.default.temporaryDirectory, environment: [:])
    }

    func testConversationStreamsDeltasFromFakeServerAndReusesTheThread() async throws {
        let convo = conversation(try fakeServer(deltas: ["Ho", "la"]))
        defer { convo.shutdown() }
        var events: [ProviderEvent] = []
        for await e in convo.send(prompt: "hola") { events.append(e) }
        XCTAssertEqual(events, [.session("th1"), .delta("Ho"), .delta("la"), .done])
        var again: [ProviderEvent] = []
        for await e in convo.send(prompt: "otra") { again.append(e) }
        XCTAssertEqual(again, [.delta("Ho"), .delta("la"), .done], "el segundo turno reutiliza el servidor y el hilo")
    }

    func testConversationReportsAuthFailure() async throws {
        let convo = conversation(try fakeServer(deltas: [], failOnTurn: true))
        defer { convo.shutdown() }
        var events: [ProviderEvent] = []
        for await e in convo.send(prompt: "hola") { events.append(e) }
        guard case .failure(let f)? = events.last else { return XCTFail("expected failure") }
        XCTAssertEqual(f.kind, .auth)
    }

    func testServerThatDiesBeforeAnswerEndsWithCliFailure() async throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("fake-codex-\(UUID().uuidString).sh")
        try "#!/bin/sh\nexit 1\n".write(to: url, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: url.path)
        let convo = conversation(url)
        defer { convo.shutdown() }
        var events: [ProviderEvent] = []
        let started = Date()
        for await e in convo.send(prompt: "hola") { events.append(e) }
        guard case .failure(let f)? = events.last else { return XCTFail("expected failure") }
        XCTAssertEqual(f.kind, .cli)
        XCTAssertLessThan(Date().timeIntervalSince(started), 10, "no debe esperar al timeout")
    }

    // MARK: - cancel (fixes from the final review)

    func testCancelAfterTheTurnStartedSendsInterruptAndEndsWithDone() async throws {
        let convo = conversation(try fakeServer(deltas: ["par"], holdTurn: true))
        defer { convo.shutdown() }
        let guardTask = Task { try? await Task.sleep(for: .seconds(8)); convo.shutdown() }   // never hang the suite
        defer { guardTask.cancel() }
        let stream = convo.send(prompt: "x")
        Task { try? await Task.sleep(for: .milliseconds(900)); convo.cancel() }
        var events: [ProviderEvent] = []
        for await e in stream { events.append(e) }
        XCTAssertTrue(events.contains(.delta("par")))
        XCTAssertEqual(events.last, .done)
    }

    func testCancelBeforeTheTurnIdIsKnownIsNotLost() async throws {
        let convo = conversation(try fakeServer(deltas: ["par"], holdTurn: true, delayStart: 0.7))
        defer { convo.shutdown() }
        let guardTask = Task { try? await Task.sleep(for: .seconds(8)); convo.shutdown() }
        defer { guardTask.cancel() }
        let stream = convo.send(prompt: "x")
        Task { try? await Task.sleep(for: .milliseconds(250)); convo.cancel() }   // turn/start still unanswered
        var events: [ProviderEvent] = []
        let started = Date()
        for await e in stream { events.append(e) }
        XCTAssertEqual(events.last, .done, "el Detener pulsado antes de conocer el turno no debe perderse")
        XCTAssertLessThan(Date().timeIntervalSince(started), 7)
    }
}
