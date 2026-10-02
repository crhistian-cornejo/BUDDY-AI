import XCTest

/// Only a no-credits error moves a turn to the fallback model (subscription.rs: `only_a_no_credits_error_triggers_the_fallback`,
/// `the_fallback_line_names_both_models`), once, with a session of its own, and nothing is stored.
final class CreditFallbackDetectionTests: XCTestCase {
    func testOnlyANoCreditsErrorCountsAsOne() {
        let yes = ["Usage limit reached", "Usage limit", "Claude AI usage limit reached|1791164280",
                   "5-hour limit reached ∙ resets 3pm", "Weekly limit reached", "You've hit your limit",
                   "You exceeded your current quota, please check your plan", "Your credit balance is too low to access the API",
                   "usageLimitExceeded", "insufficient_quota", "Out of credits",
                   "Error: You've hit your usage limit. Upgrade to Pro", "You have run out of credits", "No credits left"]
        for text in yes {
            XCTAssertTrue(CreditFallback.isNoCredits(text), text)
            XCTAssertTrue(ProviderFailure(kind: .limit, message: text).isNoCredits, text)
        }
        let no = ["Not logged in · Please run /login", "Rate limit reached for requests (429)",
                  "rate_limit_error: too many requests", "Overloaded", "context length limit exceeded", "timeout",
                  "Codex se cerró.", "", "panic at config.toml token=abc", "invalid model",
                  "network error: connection reset", "unauthorized", "El cliente de Codex no respondió a tiempo."]
        for text in no {
            XCTAssertFalse(CreditFallback.isNoCredits(text), text)
        }
    }

    func testARateLimitThatMentionsUsageLimitIsStillNoCredits() {
        XCTAssertTrue(CreditFallback.isNoCredits("rate limit: you reached your usage limit"))
    }

    func testTooManyRequestsHasItsOwnMessageAndIsNeverRetried() {
        for text in ["Rate limit reached for requests (429)", "rate_limit_error", "Too many requests", "Overloaded"] {
            XCTAssertTrue(CreditFallback.isTooManyRequests(text), text)
            XCTAssertFalse(CreditFallback.isNoCredits(text), text)
            XCTAssertEqual(ProviderFailure(kind: .limit, message: text).userSummary,
                           "Demasiadas peticiones · intenta de nuevo en un momento", text)
        }
        XCTAssertFalse(CreditFallback.isTooManyRequests("Usage limit reached"))
        XCTAssertFalse(CreditFallback.isTooManyRequests("context length limit exceeded"))
    }

    func testTheNoticeNamesBothModels() {
        XCTAssertEqual(CreditFallback.notice(primary: "GPT-6.1-Sol", fallback: "Sonnet 5.5"),
                       "Sin créditos de GPT-6.1-Sol: respondo con Sonnet 5.5")
        XCTAssertEqual(CreditFallback.notice(primary: "Sonnet 5.5", fallback: "Haiku 4.5"),
                       "Sin créditos de Sonnet 5.5: respondo con Haiku 4.5")
        XCTAssertEqual(CreditFallback.keyPrefix, "fb-")
    }
}

/// Plays a different script on every call and remembers how each call was made.
final class ScriptedRunner: ProviderRunner, @unchecked Sendable {
    struct Call: Equatable {
        var prompt: String
        var provider: ProviderID
        var sessionId: String?
        var key: String
        var modelOverride: String?
        var agentID: String
    }

    private let lock = NSLock()
    private var scripts: [[ProviderEvent]]
    private var recorded: [Call] = []

    init(_ scripts: [[ProviderEvent]]) { self.scripts = scripts }

    var calls: [Call] { lock.withLock { recorded } }

    func run(prompt: String, agent: AgentDefinition, provider: ProviderID, sessionId: String?) -> ProviderRun {
        run(prompt: prompt, agent: agent, provider: provider, sessionId: sessionId, key: agent.id, images: [])
    }

    func run(prompt: String, agent: AgentDefinition, provider: ProviderID, sessionId: String?, key: String,
             images: [String]) -> ProviderRun {
        let script: [ProviderEvent] = lock.withLock {
            recorded.append(Call(prompt: prompt, provider: provider, sessionId: sessionId, key: key,
                                 modelOverride: agent.modelOverride?.id, agentID: agent.id))
            return scripts.isEmpty ? [.done] : scripts.removeFirst()
        }
        let (stream, continuation) = AsyncStream<ProviderEvent>.makeStream()
        for event in script { continuation.yield(event) }
        continuation.finish()
        return ProviderRun(events: stream, cancel: {})
    }
}

@MainActor
final class CreditFallbackChatTests: XCTestCase {
    private var root: URL!
    private var store: AgentStore!

    private let noCredits = ProviderEvent.failure(ProviderFailure(kind: .limit, message: "You've hit your usage limit"))

    override func setUp() async throws {
        root = FileManager.default.temporaryDirectory.appendingPathComponent("fallback-\(UUID().uuidString)")
        store = AgentStore(root: root)
        try store.installDefaults()
    }

    override func tearDown() async throws { try? FileManager.default.removeItem(at: root) }

    private func agent(_ id: String) throws -> AgentDefinition {
        try XCTUnwrap(store.loadAll().first { $0.id == id })
    }

    private func ask(_ chat: AgentChat, _ text: String) async {
        chat.send(text)
        for _ in 0..<400 { if !chat.isStreaming { return }; try? await Task.sleep(for: .milliseconds(10)) }
        XCTFail("la respuesta no terminó")
    }

    func testMikaWithoutCreditsIsAnsweredByHaikuOnce() async throws {
        let runner = ScriptedRunner([[noCredits], [.session("fb-session"), .delta("Hola"), .done]])
        let chat = AgentChat(agent: try agent("mika"), store: store, runner: runner)
        await ask(chat, "hola")
        XCTAssertEqual(runner.calls.count, 2, "one retry, never more")
        XCTAssertNil(runner.calls[0].modelOverride, "the first attempt is on the primary")
        XCTAssertEqual(runner.calls[0].provider, .claude)
        XCTAssertEqual(runner.calls[1].modelOverride, "claude-haiku-4-5-20251001")
        XCTAssertEqual(runner.calls[1].provider, .claude)
        XCTAssertEqual(runner.calls[1].key, "fb-mika", "a session key of its own")
        XCTAssertNil(runner.calls[1].sessionId)
        XCTAssertNil(chat.failure, "the primary's error is not shown when the retry happens")
        XCTAssertEqual(chat.fallbackNote, "Sin créditos de Sonnet 5.5: respondo con Haiku 4.5")
        XCTAssertEqual(chat.messages.map(\.content), ["hola", "Hola"], "the question is saved once")
    }

    func testAnotherAgentFallsFromGPTToSonnet() async throws {
        let runner = ScriptedRunner([[noCredits], [.delta("Listo"), .done]])
        let chat = AgentChat(agent: try agent("mira"), store: store, runner: runner)
        await ask(chat, "resume esto")
        XCTAssertEqual(runner.calls.map(\.provider), [.codex, .claude])
        XCTAssertEqual(runner.calls[1].modelOverride, "claude-sonnet-5-5")
        XCTAssertEqual(runner.calls[1].key, "fb-mira")
        XCTAssertEqual(chat.fallbackNote, "Sin créditos de GPT-6.1-Sol: respondo con Sonnet 5.5")
        XCTAssertEqual(chat.messages.last?.content, "Listo")
    }

    func testAnAgentTheUserAddedFallsToSonnetToo() async throws {
        let dir = root.appendingPathComponent("nuevo")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try "---\nid: nuevo\nname: Nuevo\nprovider: claude\n---\nEres Nuevo.".write(to: dir.appendingPathComponent("agent.md"), atomically: true, encoding: .utf8)
        let runner = ScriptedRunner([[noCredits], [.delta("ok"), .done]])
        let chat = AgentChat(agent: try agent("nuevo"), store: store, runner: runner)
        await ask(chat, "hola")
        XCTAssertEqual(runner.calls.map(\.provider), [.codex, .claude], "it runs on Codex whatever the file says")
        XCTAssertEqual(runner.calls[1].modelOverride, "claude-sonnet-5-5")
    }

    func testThePartialAnswerIsKeptAndNothingIsRetried() async throws {
        let runner = ScriptedRunner([[.delta("Parcial"), noCredits]])
        let chat = AgentChat(agent: try agent("mika"), store: store, runner: runner)
        await ask(chat, "hola")
        XCTAssertEqual(runner.calls.count, 1)
        XCTAssertEqual(chat.messages.last?.content, "Parcial")
        XCTAssertNotNil(chat.failure, "the error is shown as before")
        XCTAssertNil(chat.fallbackNote)
    }

    func testAnyOtherFailureIsShownAndNotRetried() async throws {
        for message in ["Rate limit reached for requests (429)", "Not logged in · Please run /login", "context length limit exceeded", "timeout"] {
            let runner = ScriptedRunner([[.failure(ProviderFailure(kind: ProviderFailure.classify(message), message: message))]])
            let chat = AgentChat(agent: try agent("mika"), store: store, runner: runner)
            await ask(chat, "hola")
            XCTAssertEqual(runner.calls.count, 1, message)
            XCTAssertEqual(chat.failure?.message, message)
            XCTAssertNil(chat.fallbackNote, message)
        }
    }

    func testTheFallbackFailingToo_ShowsItsErrorAndDoesNotRetryAgain() async throws {
        let runner = ScriptedRunner([[noCredits], [noCredits]])
        let chat = AgentChat(agent: try agent("mika"), store: store, runner: runner)
        await ask(chat, "hola")
        XCTAssertEqual(runner.calls.count, 2, "at most one retry per turn")
        XCTAssertNotNil(chat.failure)
    }

    func testNoRetryWhenTheFallbackSubscriptionIsNotConnected() async throws {
        let runner = ScriptedRunner([[noCredits], [.delta("no debería"), .done]])
        let chat = AgentChat(agent: try agent("mira"), store: store, runner: runner)
        chat.fallbackAllowed = { $0 != .claude }
        await ask(chat, "hola")
        XCTAssertEqual(runner.calls.count, 1)
        XCTAssertEqual(chat.failure?.isNoCredits, true)
    }

    func testTheNextTurnTriesThePrimaryAgainAndTheNoteGoesAway() async throws {
        let runner = ScriptedRunner([[.session("s1"), .delta("uno"), .done],
                                     [noCredits], [.session("fb"), .delta("dos"), .done],
                                     [.session("s2"), .delta("tres"), .done]])
        let chat = AgentChat(agent: try agent("mika"), store: store, runner: runner)
        await ask(chat, "primero")
        await ask(chat, "segundo")
        XCTAssertEqual(chat.fallbackNote, "Sin créditos de Sonnet 5.5: respondo con Haiku 4.5")
        let calls = runner.calls
        XCTAssertEqual(calls[1].prompt, "segundo", "the primary resumes its session with just the question")
        XCTAssertEqual(calls[1].sessionId, "s1")
        XCTAssertTrue(calls[2].prompt.contains("primero") && calls[2].prompt.hasSuffix("segundo"),
                      "the retry's session is new, so it carries the recent conversation as data")
        XCTAssertNil(calls[2].sessionId)

        await ask(chat, "tercero")
        XCTAssertNil(chat.fallbackNote, "cleared on the next question")
        let third = runner.calls[3]
        XCTAssertNil(third.modelOverride, "nothing was stored: the primary is tried again")
        XCTAssertNil(third.sessionId, "the primary's session never saw the fallback's answer: a fresh one, seeded")
        XCTAssertTrue(third.prompt.contains("segundo") && third.prompt.contains("dos"))
    }

    func testANewChatClearsTheNote() async throws {
        let runner = ScriptedRunner([[noCredits], [.delta("ok"), .done]])
        let chat = AgentChat(agent: try agent("mika"), store: store, runner: runner)
        await ask(chat, "hola")
        XCTAssertNotNil(chat.fallbackNote)
        chat.clear()
        XCTAssertNil(chat.fallbackNote)
    }

    func testAHandOffFallsBackLikeAnyTurn() async throws {
        // MIKA passes the request to MIRA; MIRA's run has no credits, so it is retried on Sonnet.
        let runner = ScriptedRunner([[.delta("[[pasar:mira]] resume")], [noCredits], [.delta("Resumen"), .done]])
        let hub = AgentChatHub(store: store, runner: runner)
        let chat = hub.chat(for: "mika")
        await ask(chat, "resume este texto")
        let calls = runner.calls
        XCTAssertEqual(calls.count, 3)
        XCTAssertEqual(calls[1].agentID, "mira")
        XCTAssertEqual(calls[1].provider, .codex)
        XCTAssertEqual(calls[2].agentID, "mira")
        XCTAssertEqual(calls[2].provider, .claude)
        XCTAssertEqual(calls[2].modelOverride, "claude-sonnet-5-5")
        XCTAssertEqual(calls[2].key, "fb-handoff-mira")
        XCTAssertNil(chat.failure)
        XCTAssertEqual(chat.messages.last?.content, "Resumen")
    }
}
