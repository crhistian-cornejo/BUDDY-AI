import XCTest

final class ClaudeStreamParserTests: XCTestCase {
    private func run(_ name: String) -> [ProviderEvent] {
        var parser = ClaudeStreamParser()
        return Fixture.lines(name).flatMap { parser.feed($0) }
    }

    func testFixturesAreFound() {
        XCTAssertFalse(Fixture.lines("claude-basic.ndjson").isEmpty, "no se encontró el fixture compartido")
    }

    func testBasicStreamProducesSessionDeltasAndDone() {
        XCTAssertEqual(run("claude-basic.ndjson"), [.session("sess-1"), .delta("Hola"), .delta(", mundo"), .done])
    }

    func testUsageLimitResult() {
        let events = run("claude-limit.ndjson")
        XCTAssertEqual(events.first, .session("sess-2"))
        guard case .failure(let f) = events.last else { return XCTFail("expected failure") }
        XCTAssertEqual(f.kind, .limit)
        XCTAssertTrue(f.message.contains("resets 3:45pm"))
    }

    func testFullAssistantMessageIsUsedWhenNoPartialsArrived() {
        var parser = ClaudeStreamParser()
        let line = #"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Entero"},{"type":"tool_use","name":"Read"}]}}"#
        XCTAssertEqual(parser.feed(line), [.delta("Entero")])
    }

    func testToolUseStartAndGarbageLines() {
        var parser = ClaudeStreamParser()
        XCTAssertEqual(parser.feed(#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"tool_use","name":"WebSearch"}}}"#),
                       [.tool(name: "WebSearch", summary: "")])
        XCTAssertEqual(parser.feed("no es json"), [])
        XCTAssertEqual(parser.feed(#"{"sin":"tipo"}"#), [])
        XCTAssertEqual(parser.feed(""), [])
    }
}
