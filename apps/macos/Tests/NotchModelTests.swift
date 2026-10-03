import XCTest
@testable import Buddy

final class NotchModelTests: XCTestCase {
    /// «Permitir» waits until the whole command of an approval card has been in view.
    func testACommandIsSeenWholeWhenItFitsOrWasScrolledToItsEnd() {
        XCTAssertTrue(NotchLayout.seenWhole(offset: 0, box: 96, content: 96))
        XCTAssertTrue(NotchLayout.seenWhole(offset: 0, box: 96, content: 60))
        XCTAssertFalse(NotchLayout.seenWhole(offset: 0, box: 96, content: 400))
        XCTAssertFalse(NotchLayout.seenWhole(offset: 200, box: 96, content: 400))
        XCTAssertTrue(NotchLayout.seenWhole(offset: 304, box: 96, content: 400))
        XCTAssertTrue(NotchLayout.seenWhole(offset: 302.5, box: 96, content: 400))
        XCTAssertFalse(NotchLayout.seenWhole(offset: 0, box: 0, content: 0))
    }

    @MainActor
    func testSessionFlagStartsUnlockSynchronouslyAndOnlyOnce() {
        var locked: Bool? = true
        let state = NotchLockState()
        let monitor = NotchUnlockMonitor(readLocked: { locked })
        var events = 0
        monitor.onUnlock = { events += 1; state.unlock() }
        state.lock()
        monitor.start()
        locked = nil // A missing session/user switch is not an authenticated unlock.
        monitor.check()
        XCTAssertEqual(state.phase, .locked)
        locked = false
        monitor.check()
        XCTAssertEqual(state.phase, .unlocking) // No task, geometry reset or delayed notification.
        monitor.check()
        XCTAssertEqual(events, 1)
        state.stop()
        locked = true
        monitor.start()
        monitor.stop()
        locked = false
        monitor.check()
        XCTAssertEqual(events, 1)
    }

    @MainActor
    func testLockAnimationRequiresLockAndIgnoresDuplicateUnlocks() async throws {
        let state = NotchLockState()
        var finishes = 0
        state.onFinish = { finishes += 1 }
        state.unlock(seconds: 0.02)
        XCTAssertEqual(state.phase, .hidden)
        state.lock()
        state.unlock(seconds: 0.03)
        state.unlock(seconds: 10)
        XCTAssertEqual(state.phase, .unlocking)
        try await Task.sleep(for: .milliseconds(80))
        XCTAssertEqual(state.phase, .hidden)
        XCTAssertEqual(finishes, 1)
    }

    @MainActor
    func testRelockingAndStoppingCancelUnlockDismissal() async throws {
        let state = NotchLockState()
        var finishes = 0
        state.onFinish = { finishes += 1 }
        state.lock()
        state.unlock(seconds: 0.02)
        state.lock()
        try await Task.sleep(for: .milliseconds(60))
        XCTAssertEqual(state.phase, .locked)
        XCTAssertEqual(finishes, 0)
        state.unlock(seconds: 0.02)
        state.stop()
        try await Task.sleep(for: .milliseconds(60))
        XCTAssertEqual(state.phase, .hidden)
        XCTAssertEqual(finishes, 0)
    }

    func testMediaKeysConsumeOnlySupportedActionsAndPreserveShortcuts() {
        func decode(_ code: Int, state: Int = 0x0a, option: Bool = false, shift: Bool = false,
                    command: Bool = false, control: Bool = false) -> NotchMediaKey.Press? {
            NotchMediaKey.decode(data: code << 16 | state << 8, option: option, shift: shift, command: command, control: control)
        }
        XCTAssertEqual(decode(0)?.action, .volumeUp)
        XCTAssertEqual(decode(1)?.action, .volumeDown)
        XCTAssertEqual(decode(7)?.action, .mute)
        XCTAssertEqual(decode(2)?.action, .brightnessUp)
        XCTAssertEqual(decode(3)?.action, .brightnessDown)
        XCTAssertEqual(decode(0, state: 0x0b)?.down, false)
        XCTAssertEqual(decode(0, option: true, shift: true)?.fine, true)
        XCTAssertNil(decode(0, option: true))
        XCTAssertNil(decode(0, command: true))
        XCTAssertNil(decode(0, control: true))
        XCTAssertNil(decode(16)) // Play/pause must keep reaching the player.
        XCTAssertNil(decode(0, state: 0))
    }

    func testFocusUnavailableDataNeverBecomesAnOffState() throws {
        XCTAssertNil(NotchFocusReading.fromAssertions(Data("{}".utf8)))
        XCTAssertNil(NotchFocusReading.fromAssertions(Data("unreadable".utf8)))
        let off = Data(#"{"data":[{"storeAssertionRecords":[]}]}"#.utf8)
        let on = Data(#"{"data":[{"storeAssertionRecords":[{"assertionDetails":{"modeIdentifier":"com.apple.donotdisturb.mode.default"}}]}]}"#.utf8)
        XCTAssertEqual(NotchFocusReading.fromAssertions(off)?.active, false)
        XCTAssertEqual(NotchFocusReading.fromAssertions(on), .init(active: true, title: "No molestar"))
    }

    @MainActor
    func testSystemStatesAndOrdinaryNotificationsStayCollapsedButApprovalsExpand() {
        let model = NotchModel()
        let notch = CGSize(width: 190, height: 32)
        model.show(.init(kind: .finished, agent: "buddy", title: "Listo", detail: "Resultado"))
        XCTAssertEqual(model.mode, .idle)
        guard case .notice = model.ear else { return XCTFail("Show the compact notice") }
        model.showStatus(.init(kind: .brightness, title: "Pantalla", symbol: "sun.max.fill", level: 0.5))
        XCTAssertEqual(model.mode, .idle)
        XCTAssertEqual(NotchLayout.size(model, notch: notch), CGSize(width: 414, height: 32))
        guard case .system = model.ear else { return XCTFail("Latest system state takes the ears") }
        model.setHovering(true)
        XCTAssertEqual(model.mode, .notice)
        model.setHovering(false)
        XCTAssertEqual(model.mode, .idle)
        model.show(.init(kind: .approval(requestID: "approval", canAllow: true), agent: "codex", title: "Permiso", detail: ""))
        XCTAssertEqual(model.mode, .notice)
        XCTAssertGreaterThan(NotchLayout.size(model, notch: notch).height, notch.height)
    }

    @MainActor
    func testNewSystemChangeReplacesAndExtendsThePreviousStatus() async throws {
        let model = NotchModel()
        model.showStatus(.init(kind: .volume, title: "Volumen", symbol: "speaker.fill", level: 0.2), seconds: 0.03)
        let brightness = NotchStatus(kind: .brightness, title: "Pantalla", symbol: "sun.max.fill", level: 0.6)
        model.showStatus(brightness, seconds: 0.15)
        try await Task.sleep(for: .milliseconds(70))
        XCTAssertEqual(model.status, brightness)
        try await Task.sleep(for: .milliseconds(130))
        XCTAssertNil(model.status)
        XCTAssertEqual(model.mode, .idle)
    }

    func testCompactUsageChoosesRequestedWindowsRegardlessOfOrder() {
        let weekly = UsageWindow(label: "semana", usedPct: 20, resetsAt: nil, seenAt: 0)
        let fiveHour = UsageWindow(label: "5 h", usedPct: 6, resetsAt: nil, seenAt: 0)
        let claude = ProviderUsage(provider: "claude", name: "Claude", windows: [weekly, fiveHour])
        let codex = ProviderUsage(provider: "codex", name: "Codex", windows: [fiveHour, weekly])
        XCTAssertEqual(NotchUsage.window(for: claude), fiveHour)
        XCTAssertEqual(NotchUsage.window(for: codex), weekly)
        XCTAssertEqual(NotchUsage.help(plan: codex, window: weekly), "Codex · semana · 20 % usado")
    }

    func testCompactUsageFallsBackToAvailableWindowAndOmitsEmptyPlans() {
        let monthly = UsageWindow(label: "mes", usedPct: 35, resetsAt: nil, seenAt: 0)
        let gemini = ProviderUsage(provider: "antigravity", name: "Gemini", windows: [monthly])
        let empty = ProviderUsage(provider: "codex", name: "Codex", windows: [])
        XCTAssertEqual(NotchUsage.window(for: gemini), monthly)
        XCTAssertNil(NotchUsage.window(for: empty))
    }

    @MainActor
    func testPinnedToolsStayOpenAfterPointerLeaves() {
        let model = NotchModel()
        model.setHovering(true)
        model.pinned = true
        model.setHovering(false)
        XCTAssertEqual(model.mode, .open)
        model.pinned = false
        XCTAssertEqual(model.mode, .idle)
    }

    @MainActor
    func testClosingDoesNotReopenUntilPointerLeaves() {
        let model = NotchModel()
        model.setHovering(true)
        model.pinned = true
        model.collapse()
        XCTAssertEqual(model.mode, .idle)
        XCTAssertFalse(model.pinned)
        model.setHovering(true)
        XCTAssertEqual(model.mode, .idle)
        model.setHovering(false)
        model.setHovering(true)
        XCTAssertEqual(model.mode, .open)
    }

    @MainActor
    func testClosingPreservesApprovalAndDroppedFiles() {
        let model = NotchModel()
        let file = URL(fileURLWithPath: "/tmp/notch-review.txt")
        model.dropped = [file]
        model.show(.init(kind: .approval(requestID: "request", canAllow: true), agent: "codex", title: "Permiso", detail: ""))
        model.collapse()
        XCTAssertEqual(model.mode, .notice)
        XCTAssertEqual(model.notice?.isApproval, true)
        XCTAssertEqual(model.dropped, [file])
        model.closeApproval("request")
        XCTAssertEqual(model.mode, .idle)
        model.revealFiles()
        XCTAssertEqual(model.tab, .files)
        XCTAssertEqual(model.mode, .open)
        model.collapse()
        XCTAssertEqual(model.dropped, [file])
        XCTAssertEqual(model.mode, .idle)
    }

    @MainActor
    func testWaitingSessionTakesPriorityOverBuddyThenWorkingReturns() {
        let model = NotchModel()
        model.buddyBusy = true
        model.update(session: "session", agent: "codex", project: "Buddy", state: "waiting")
        guard case .session(let waiting) = model.ear else { return XCTFail("Waiting session should be visible") }
        XCTAssertEqual(waiting.id, "session")
        XCTAssertEqual(model.activityLabel, "Codex espera tu respuesta")
        model.update(session: "session", agent: "codex", project: "Buddy", state: "working")
        XCTAssertEqual(model.ear, .buddy)
        model.buddyBusy = false
        guard case .session = model.ear else { return XCTFail("Working session should be visible") }
        model.update(session: "session", agent: "codex", project: "Buddy", state: "ended")
        XCTAssertEqual(model.ear, .none)
    }
}
