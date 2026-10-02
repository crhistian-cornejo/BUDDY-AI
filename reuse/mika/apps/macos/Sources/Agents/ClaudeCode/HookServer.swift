import Foundation
import Darwin
import AppKit

// MARK: - HookServer
// Listens on a Unix domain socket for events from mika-hook (Claude Code and Codex hooks).
// Thread-safe: socket I/O on background threads, state updates dispatched to main queue.
//
// Both agents run the same relay: Claude Code as `mika-hook`, Codex as `mika-hook --codex`
// (see CodexHooksInstaller). The relay tags every payload `_agent: "claude" | "codex"` and that
// tag alone picks the pill: Claude Code's "integration_claude" (VS Code sessions only) or
// Codex's "integration_codex" (any terminal).
//
// Hardening: the support directory is 0700 and the socket 0600 (umask 077 around bind);
// a client must run as the same user (getpeereid); a request is capped at 1 MB and must
// arrive within 5 s; at most 16 connections are read at the same time. Whenever something
// goes wrong the relay gets no decision, prints nothing, and Claude Code carries on with
// its own permission prompt.

final class HookServer: @unchecked Sendable {
    static let shared = HookServer()

    // Support directory paths
    static var supportDir: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Mika")
    }
    static var socketPath: String { supportDir.appendingPathComponent("mika.sock").path }
    static var hookScriptPath: String { supportDir.appendingPathComponent("mika-hook").path }
    static var gateScriptPath: String { supportDir.appendingPathComponent("mika-gate").path }

    /// Largest request accepted from the relay (the relay applies the same cap).
    static let maxRequestBytes = 1_048_576
    /// A client must send its whole request within this many seconds.
    private static let readTimeoutSeconds = 5
    /// Connections being read at the same time; extra ones are closed right away.
    private static let maxConcurrentConnections = 16

    /// The two session pills.
    static let claudeTaskId = "integration_claude"
    static let codexTaskId = "integration_codex"

    private var serverFD: Int32 = -1
    private var pendingApprovalFD: Int32 = -1   // held open while user decides
    private var pendingApprovalTaskId: String? = nil  // the pill whose request is on the card
    private var activeSessionId: String? = nil  // current agent session

    private let connectionLock = NSLock()
    private var activeConnections = 0

    private init() {}

    // MARK: - Start

    func start() {
        installHookScript()
        installGateScript()
        Thread.detachNewThread { self.serverThread() }
    }

    /// Creates the support directory (socket, relay script, drop inbox) and forces mode 0700.
    @discardableResult
    static func ensureSupportDir() -> Bool {
        let path = supportDir.path
        let fm = FileManager.default
        do {
            try fm.createDirectory(atPath: path, withIntermediateDirectories: true,
                                   attributes: [.posixPermissions: NSNumber(value: 0o700)])
            try fm.setAttributes([.posixPermissions: NSNumber(value: 0o700)], ofItemAtPath: path)
            return true
        } catch {
            MikaLog.info("Support directory unavailable: \(error.localizedDescription)")
            return false
        }
    }

    // MARK: - Socket server (background thread)

    private func serverThread() {
        guard Self.ensureSupportDir() else { return }
        let path = Self.socketPath

        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let pathBytes = Array(path.utf8)
        let capacity = MemoryLayout.size(ofValue: addr.sun_path)
        guard pathBytes.count < capacity else {
            MikaLog.info("Socket path too long (\(pathBytes.count) bytes); Claude Code hooks disabled")
            return
        }
        withUnsafeMutableBytes(of: &addr.sun_path) { raw in
            for (i, byte) in pathBytes.enumerated() { raw[i] = byte }
            raw[pathBytes.count] = 0
        }

        unlink(path)
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { return }
        serverFD = fd

        // Create the socket file as 0600 (umask 077 during bind): only this user may connect.
        let previousMask = umask(0o077)
        let bindRC = withUnsafePointer(to: &addr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        umask(previousMask)
        guard bindRC == 0 else { close(fd); serverFD = -1; return }
        chmod(path, 0o600)
        guard Darwin.listen(fd, 16) == 0 else { close(fd); serverFD = -1; return }

        while true {
            let clientFD = Darwin.accept(fd, nil, nil)
            if clientFD < 0 {
                let err = errno
                if err == EINTR || err == ECONNABORTED { continue }
                if err == EMFILE || err == ENFILE { usleep(200_000); continue }
                MikaLog.info("Hook server stopped (accept errno \(err))")
                break
            }
            // Same user only, and a bounded number of connections being read at once.
            guard peerIsCurrentUser(clientFD), reserveConnection() else {
                close(clientFD)
                continue
            }
            configureClientSocket(clientFD)
            Thread.detachNewThread {
                self.handleClient(fd: clientFD)
                self.releaseConnection()
            }
        }
    }

    /// The peer process must run as the same user as MIKA.
    private func peerIsCurrentUser(_ fd: Int32) -> Bool {
        var uid: uid_t = 0
        var gid: gid_t = 0
        guard getpeereid(fd, &uid, &gid) == 0 else { return false }
        return uid == getuid()
    }

    /// Read/write timeouts so a silent client cannot hold a thread forever; no SIGPIPE.
    private func configureClientSocket(_ fd: Int32) {
        var timeout = timeval(tv_sec: Self.readTimeoutSeconds, tv_usec: 0)
        let timeoutSize = socklen_t(MemoryLayout<timeval>.size)
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, timeoutSize)
        setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, timeoutSize)
        var on: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
    }

    private func reserveConnection() -> Bool {
        connectionLock.withLock {
            guard activeConnections < Self.maxConcurrentConnections else { return false }
            activeConnections += 1
            return true
        }
    }

    private func releaseConnection() {
        connectionLock.withLock { activeConnections -= 1 }
    }

    // MARK: - Client handler (background thread)

    private func handleClient(fd: Int32) {
        guard let raw = readRequest(fd: fd),
              let payload = (try? JSONSerialization.jsonObject(with: raw)) as? [String: Any] else {
            // Oversized, timed out or not JSON: no decision, the relay prints nothing.
            sendLine(fd: fd, text: #"{"ok":true}"#)
            close(fd)
            return
        }

        let eventName = payload["hook_event_name"] as? String ?? ""

        if eventName == "PreToolUse", payload["_gate"] != nil {
            // An agent of MIKA wants to run a command: held open until the user clicks (or the gate refuses).
            Task.detached { [weak self] in
                let verdict = await AgentGate.judge(payload: payload)
                self?.sendLine(fd: fd, text: AgentGate.replyLine(verdict))
                close(fd)
            }
        } else if eventName == "PermissionRequest" {
            // Hold fd open — Claude Code waits for our decision (up to 120s)
            Task { @MainActor in self.processPermissionRequest(fd: fd, payload: payload) }
        } else {
            Task { @MainActor in self.processEvent(name: eventName, payload: payload) }
            sendLine(fd: fd, text: #"{"ok":true}"#)
            close(fd)
        }
    }

    /// Reads one newline-terminated request. Nil on timeout, error, empty or oversized request.
    private func readRequest(fd: Int32) -> Data? {
        var raw = Data()
        var buf = [UInt8](repeating: 0, count: 4096)
        while true {
            let n = recv(fd, &buf, buf.count, 0)
            if n < 0 { return nil }      // SO_RCVTIMEO expired, or a socket error
            if n == 0 { break }          // peer closed: use what we have
            if let newline = buf[0..<n].firstIndex(of: UInt8(ascii: "\n")) {
                raw.append(contentsOf: buf[0..<newline])
                break
            }
            raw.append(contentsOf: buf[0..<n])
            if raw.count > Self.maxRequestBytes { return nil }
        }
        return (raw.isEmpty || raw.count > Self.maxRequestBytes) ? nil : raw
    }


    // MARK: - Event → AppState
    // Claude Code events route to the permanent "integration_claude" task, Codex events to
    // "integration_codex" (see `taskId(for:)`).
    // View switches only happen if that pill is the currently focused mika.
    // When not focused: state updates animate the mini bot in the pill; badge shown for alerts.

    /// The pill a payload belongs to, from the relay's `_agent` tag (missing = Claude Code).
    static func taskId(for payload: [String: Any]) -> String {
        (payload["_agent"] as? String) == "codex" ? codexTaskId : claudeTaskId
    }

    /// Claude Code's pill is the VS Code pill: only sessions running in VS Code reach it.
    /// Codex has a pill of its own and is shown from any terminal.
    private static func isShown(_ payload: [String: Any], taskId: String) -> Bool {
        guard taskId == claudeTaskId else { return true }
        let termProgram = payload["term_program"] as? String ?? ""
        let bundleId    = payload["bundle_id"]    as? String ?? ""
        return termProgram.lowercased().contains("vscode") || bundleId.lowercased().contains("vscode")
    }

    /// The Codex pill only loads once its hooks are installed; an event proves they are.
    @MainActor
    private func ensurePill(_ taskId: String) {
        let state = AppState.shared
        guard taskId == Self.codexTaskId, !state.tasks.contains(where: { $0.id == taskId }) else { return }
        state.tasks.append(AgentTask.codexPill)
    }

    @MainActor
    private func processEvent(name: String, payload: [String: Any]) {
        let state = AppState.shared
        let sessionId = payload["session_id"] as? String ?? "unknown"
        let cwd = payload["cwd"] as? String ?? ""
        let rawName = URL(fileURLWithPath: cwd).lastPathComponent
        let projectName = aliasProjectName(rawName.isEmpty ? "Session" : rawName)
        let taskId = Self.taskId(for: payload)

        guard Self.isShown(payload, taskId: taskId) else {
            let termProgram = payload["term_program"] as? String ?? ""
            let bundleId    = payload["bundle_id"]    as? String ?? ""
            MikaLog.debug("Ignored \(name) from \(termProgram.isEmpty ? bundleId : termProgram)")
            return
        }
        ensurePill(taskId)

        let focused = state.focusId == taskId

        switch name {

        case "SessionStart":
            activeSessionId = sessionId
            upsertTask(id: taskId, projectName: projectName, cwd: cwd)
            MikaLog.info("SessionStart (\(sessionId.prefix(8)))")
            if state.isPresent { expandIfNeeded(to: .overview) }
            SoundEngine.shared.play("work")

        case "UserPromptSubmit":
            activeSessionId = sessionId
            upsertTask(id: taskId, projectName: projectName, cwd: cwd)
            state.updateTask(id: taskId, state: .thinking)
            if let prompt = payload["prompt"] as? String, !prompt.isEmpty {
                // The goal is set before the step is appended: the step announces "Claude is working on <goal>".
                if let idx = state.tasks.firstIndex(where: { $0.id == taskId }) {
                    state.tasks[idx].goal = SessionText.goal(from: prompt)
                }
                appendStep(id: taskId, step: String(prompt.prefix(60)))
            }
            if state.isPresent { expandIfNeeded(to: .overview) }

        case "PreToolUse":
            activeSessionId = sessionId
            upsertTask(id: taskId, projectName: projectName, cwd: cwd)
            state.updateTask(id: taskId, state: .working)
            let tool = payload["tool_name"] as? String ?? "Tool"
            let input = payload["tool_input"] as? [String: Any] ?? [:]
            let step = stepLabel(tool: tool, input: input)
            appendStep(id: taskId, step: step)
            // Tool inputs may contain secrets: log the tool name only (redacted excerpt in DEBUG).
            MikaLog.info("PreToolUse \(tool)")
            MikaLog.debug("PreToolUse \(MikaLog.redacted(step))")

        case "PostToolUse":
            state.updateTask(id: taskId, state: .working)

        case "PostToolUseFailure":
            state.updateTask(id: taskId, state: .working)
            appendStep(id: taskId, step: "⚠ failed")

        case "Notification":
            let message = payload["message"] as? String ?? ""
            if message.lowercased().contains("rate limit") {
                state.updateTask(id: taskId, state: .ratelimit)
                SoundEngine.shared.play("rate")
            } else if message.hasSuffix("?") {
                state.updateTask(id: taskId, state: .question)
                appendStep(id: taskId, step: message)
            }

        case "Stop":
            state.updateTask(id: taskId, state: .finished)
            // Codex calls the turn's last message `last_assistant_message`.
            let said = (payload["message"] as? String) ?? (payload["last_assistant_message"] as? String) ?? ""
            if !said.isEmpty {
                appendStep(id: taskId, step: String(said.prefix(60)))
            }
            SoundEngine.shared.play("finish")
            if focused {
                expandIfNeeded(to: .finished)
            } else {
                setPillBadge(id: taskId, badge: .finished)
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 5.2) {
                state.updateTask(id: taskId, state: .idle)
                self.clearPillBadge(id: taskId)
            }

        case "StopFailure":
            state.updateTask(id: taskId, state: .error)
            SoundEngine.shared.play("error")
            if focused {
                expandIfNeeded(to: .error)
            } else {
                setPillBadge(id: taskId, badge: .error)
            }

        case "SessionEnd":
            activeSessionId = nil
            state.updateTask(id: taskId, state: .idle)
            clearSession(id: taskId)

        case "SubagentStart":
            appendStep(id: taskId, step: "+ subagent")

        case "SubagentStop":
            appendStep(id: taskId, step: "• subagent done")

        case "Interrupt":
            // Codex only: the user stopped the turn. Nothing failed, nothing finished.
            state.updateTask(id: taskId, state: .idle)
            appendStep(id: taskId, step: "■ interrupted")

        default:
            break
        }
    }

    // MARK: - Helpers

    @MainActor
    private func expandIfNeeded(to view: IslandView) {
        let state = AppState.shared
        let isAlert: Bool
        switch view {
        case .approval, .finished, .error, .confused: isAlert = true
        default: isAlert = false
        }
        if state.mode == .expanded {
            // Only force-switch view for alerts — leave user on their current view otherwise
            if isAlert { state.view = view }
        } else if isAlert {
            // Alerts always force-expand
            NotificationCenter.default.post(name: .hookExpand, object: view)
        } else if state.mode == .hidden {
            // Non-alert work events: reveal compact only, never force-expand
            NotificationCenter.default.post(name: .hookReveal, object: nil)
        }
        // Already compact and non-alert: Mika state update is enough, no expand
    }

    // MARK: - Permission request (blocking — Claude Code waits for decision)

    @MainActor
    private func processPermissionRequest(fd: Int32, payload: [String: Any]) {
        let state = AppState.shared
        let sessionId = payload["session_id"] as? String ?? "unknown"
        let cwd       = payload["cwd"]        as? String ?? ""
        let rawName   = URL(fileURLWithPath: cwd).lastPathComponent
        let projectName = aliasProjectName(rawName.isEmpty ? "Session" : rawName)
        let taskId = Self.taskId(for: payload)

        // One card at a time: a command of an agent of MIKA is on the island, so Claude Code asks in its own terminal.
        guard !IslandGatePresenter.shared.isPending else {
            Task.detached { [weak self] in
                self?.sendLine(fd: fd, text: #"{"permissionDecision":"ask"}"#)
                close(fd)
            }
            return
        }
        guard Self.isShown(payload, taskId: taskId) else {
            Task.detached { [weak self] in
                self?.sendLine(fd: fd, text: #"{"permissionDecision":"ask"}"#)
                close(fd)
            }
            return
        }
        ensurePill(taskId)

        let approval = PermissionRequestFormatter.approvalInfo(sessionId: sessionId, payload: payload)
        // Never log the command itself (it may contain secrets).
        MikaLog.info("PermissionRequest \(approval.tool) (\(approval.command.count) chars, \(approval.rules.count) suggested rule(s))")
        MikaLog.debug("PermissionRequest \(approval.tool): \(MikaLog.redacted(approval.command))")

        if pendingApprovalFD >= 0 {
            let old = pendingApprovalFD
            Task.detached { [weak self] in
                // "ask" → mika-hook outputs nothing → the agent re-asks
                self?.sendLine(fd: old, text: #"{"permissionDecision":"ask"}"#)
                close(old)
            }
            // The replaced request may belong to the other agent's pill.
            if let oldTask = pendingApprovalTaskId, oldTask != taskId {
                state.updateTask(id: oldTask, state: .working)
                clearPillBadge(id: oldTask)
            }
        }
        pendingApprovalFD = fd
        pendingApprovalTaskId = taskId
        activeSessionId = sessionId

        upsertTask(id: taskId, projectName: projectName, cwd: cwd)
        state.updateTask(id: taskId, state: .approval)
        state.pendingApproval = approval
        state.isPinned = true
        SoundEngine.shared.play("approval")

        // Approval always forces the island open — user must be able to respond
        state.focusId = taskId
        expandIfNeeded(to: .approval)

        let captured = fd
        DispatchQueue.main.asyncAfter(deadline: .now() + 115) { [weak self] in
            guard let self, self.pendingApprovalFD == captured else { return }
            // "ask" → mika-hook outputs nothing → Claude Code re-asks rather than denying
            self.sendApprovalDecision("ask")
        }
    }

    /// Called by ApprovalView buttons. Writes the decision to the waiting mika-hook and cleans up.
    @MainActor
    func sendApprovalDecision(_ decision: String) {
        // The card may be a command of an agent of MIKA, not a Claude Code request.
        if IslandGatePresenter.shared.resolve(decision) { return }
        let fd = pendingApprovalFD
        pendingApprovalFD = -1
        let taskId = pendingApprovalTaskId ?? Self.claudeTaskId
        pendingApprovalTaskId = nil

        let json: String
        switch decision {
        case "allow":  json = #"{"permissionDecision":"allow"}"#
        case "always": json = #"{"permissionDecision":"always"}"#
        case "ask":    json = #"{"permissionDecision":"ask"}"#
        default:       json = #"{"permissionDecision":"deny"}"#
        }
        MikaLog.info("Approval decision: \(decision)")

        if fd >= 0 {
            Task.detached { [weak self] in
                self?.sendLine(fd: fd, text: json)
                close(fd)
            }
        }

        let state = AppState.shared
        state.pendingApproval = nil
        state.isPinned = false
        state.updateTask(id: taskId, state: .working)
        clearPillBadge(id: taskId)
        state.view = state.tasks.isEmpty ? .empty : .overview
    }

    /// Updates a session pill with the current session project name and cwd.
    @MainActor
    private func upsertTask(id: String, projectName: String, cwd: String = "") {
        let state = AppState.shared
        guard let idx = state.tasks.firstIndex(where: { $0.id == id }) else { return }
        state.tasks[idx].name = projectName
        if !cwd.isEmpty { state.tasks[idx].sessionCwd = cwd }
    }

    // MARK: - Badge helpers

    @MainActor
    private func setPillBadge(id: String, badge: PillBadge) {
        let state = AppState.shared
        guard let idx = state.tasks.firstIndex(where: { $0.id == id }) else { return }
        state.tasks[idx].pillBadge = badge
    }

    @MainActor
    private func clearPillBadge(id: String) {
        let state = AppState.shared
        guard let idx = state.tasks.firstIndex(where: { $0.id == id }) else { return }
        state.tasks[idx].pillBadge = nil
    }

    /// Resets a session pill to idle, clears steps and project name.
    @MainActor
    private func clearSession(id: String) {
        let state = AppState.shared
        guard let idx = state.tasks.firstIndex(where: { $0.id == id }) else { return }
        state.tasks[idx].steps = []
        state.tasks[idx].stepIndex = 0
        state.tasks[idx].goal = nil
        state.tasks[idx].name = id == Self.codexTaskId ? AgentTask.codexPill.name : "VS Code"
        state.tasks[idx].pillBadge = nil
    }

    @MainActor
    private func appendStep(id: String, step: String) {
        let state = AppState.shared
        guard let idx = state.tasks.firstIndex(where: { $0.id == id }) else { return }
        state.tasks[idx].steps.append(step)
        if state.tasks[idx].steps.count > 20 { state.tasks[idx].steps.removeFirst() }
        state.tasks[idx].stepIndex = state.tasks[idx].steps.count - 1
        // The LED board, the compact bar and the circle beside the notch say the same calm line as the ticker ("Claude is
        // working on <goal>", "Claude finished"), not the raw hook step; the same line twice within 8 s is said once.
        state.announceSession(of: state.tasks[idx])
    }

    // MARK: - Project name alias mapping

    private func aliasProjectName(_ name: String) -> String {
        let aliases: [String: String] = [:]
        return aliases[name.lowercased()] ?? name
    }

    // MARK: - Step labels (ticker)

    /// Short ticker label for a tool call, e.g. "Run · npm test" or "Edit · AppState.swift".
    private func stepLabel(tool: String, input: [String: Any]) -> String {
        let labels: [String: String] = [
            "Bash":         "Run",
            "Read":         "Read",
            "Write":        "Write",
            "Edit":         "Edit",
            "MultiEdit":    "Edit",
            "Glob":         "Find",
            "Grep":         "Search",
            "WebSearch":    "Web search",
            "WebFetch":     "Fetch",
            "TodoWrite":    "Tasks",
            "Task":         "Agent",
            "LS":           "List",
            "NotebookEdit": "Notebook",
        ]
        let label = labels[tool] ?? tool
        if let cmd = input["command"] as? String {
            return "\(label) · \(SessionText.oneLine(cmd, limit: 34))"
        } else if let path = input["path"] as? String {
            return "\(label) · \(URL(fileURLWithPath: path).lastPathComponent)"
        } else if let file = input["file_path"] as? String {
            return "\(label) · \(URL(fileURLWithPath: file).lastPathComponent)"
        } else if let query = input["query"] as? String {
            return "\(label) · \(SessionText.oneLine(query, limit: 34))"
        } else if let url = input["url"] as? String, url.lowercased().hasPrefix("http"), let host = ChatSource.host(of: url) {
            return "\(label) · \(host)"      // WebFetch: which site
        }
        return label
    }

    private func sendLine(fd: Int32, text: String) {
        let bytes = Array((text + "\n").utf8)
        bytes.withUnsafeBytes { buffer in
            var sent = 0
            while sent < buffer.count {
                let n = Darwin.send(fd, buffer.baseAddress! + sent, buffer.count - sent, 0)
                if n <= 0 { break }
                sent += n
            }
        }
    }

    // MARK: - mika-hook relay script

    /// Writes the relay script — only when its content changed — and keeps it executable.
    func installHookScript() {
        guard Self.ensureSupportDir() else { return }
        let url = URL(fileURLWithPath: Self.hookScriptPath)
        let script = Self.relayScript(socketPath: Self.socketPath)
        if (try? String(contentsOf: url, encoding: .utf8)) != script {
            do {
                try Data(script.utf8).write(to: url, options: .atomic)
            } catch {
                MikaLog.info("Could not write mika-hook: \(error.localizedDescription)")
                return
            }
        }
        try? FileManager.default.setAttributes([.posixPermissions: NSNumber(value: 0o700)],
                                               ofItemAtPath: url.path)
    }

    /// The `expandIfNeeded` an agent's command card uses to bring the island up.
    @MainActor
    func presentApprovalCard() { expandIfNeeded(to: .approval) }

    /// Writes the gate relay (like the hook relay, only when its content changed), executable.
    func installGateScript() {
        guard Self.ensureSupportDir() else { return }
        let url = URL(fileURLWithPath: Self.gateScriptPath)
        let script = Self.gateRelayScript(socketPath: Self.socketPath)
        if (try? String(contentsOf: url, encoding: .utf8)) != script {
            do { try Data(script.utf8).write(to: url, options: .atomic) } catch {
                MikaLog.info("Could not write mika-gate: \(error.localizedDescription)")
                return
            }
        }
        try? FileManager.default.setAttributes([.posixPermissions: NSNumber(value: 0o700)], ofItemAtPath: url.path)
    }

    /// Python source of the gate relay. Unlike `mika-hook` it fails CLOSED: anything but an explicit allow from MIKA is a
    /// denial of the command.
    static func gateRelayScript(socketPath: String) -> String {
        let socketLiteral = pythonStringLiteral(socketPath)
        return #"""
        #!/usr/bin/env python3
        # mika-gate: asks MIKA whether a command of one of its agents may run (a Claude PreToolUse hook on Bash).
        # Generated by MIKA and rewritten at launch whenever its content changes (edits are lost).
        # Fails CLOSED: it prints an allow only when MIKA answered allow after a click; any error, a missing token, a
        # missing app or a missing answer prints a denial.
        import json
        import os
        import socket
        import sys

        SOCKET_PATH = \#(socketLiteral)
        TIMEOUT = 118
        MAX_COMMAND = 16384


        def out(decision, reason=''):
            body = {'hookEventName': 'PreToolUse', 'permissionDecision': decision}
            if reason:
                body['permissionDecisionReason'] = reason
            print(json.dumps({'hookSpecificOutput': body}))


        def main():
            token = os.environ.get('MIKA_GATE_TOKEN', '')
            payload = json.loads(sys.stdin.buffer.read() or b'{}')
            if not token or not isinstance(payload, dict):
                return out('deny', 'MIKA no puede revisar este comando.')
            payload['_gate'] = token
            command = (payload.get('tool_input') or {}).get('command')
            if isinstance(command, str) and len(command) > MAX_COMMAND:
                payload['tool_input']['command'] = command[:MAX_COMMAND]
                payload['_truncated'] = True
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            try:
                s.settimeout(TIMEOUT)
                s.connect(SOCKET_PATH)
                s.sendall((json.dumps(payload) + '\n').encode('utf-8'))
                data = b''
                while b'\n' not in data:
                    chunk = s.recv(4096)
                    if not chunk:
                        break
                    data += chunk
            finally:
                s.close()
            reply = json.loads(data.decode('utf-8', 'replace').splitlines()[0])
            if reply.get('permissionDecision') == 'allow':
                return out('allow')
            return out('deny', reply.get('reason') or 'Denegado desde MIKA.')


        try:
            main()
        except Exception:
            out('deny', 'MIKA no respondió: el comando no se ejecutó.')
        sys.exit(0)
        """#
    }

    /// A single-quoted Python string literal holding `value`.
    private static func pythonStringLiteral(_ value: String) -> String {
        let escaped = value
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "'", with: "\\'")
            .replacingOccurrences(of: "\n", with: "\\n")
            .replacingOccurrences(of: "\r", with: "\\r")
        return "'" + escaped + "'"
    }

    /// Python source of the relay. The socket path is embedded so both sides always agree.
    static func relayScript(socketPath: String) -> String {
        let socketLiteral = pythonStringLiteral(socketPath)
        return #"""
        #!/usr/bin/env python3
        # mika-hook: relays Claude Code and Codex hook events to the MIKA app over a local Unix socket.
        # Claude Code runs `mika-hook`, Codex runs `mika-hook --codex`; every payload is tagged
        # `_agent` ("claude" / "codex") so MIKA knows whose session it is.
        #
        # Generated by MIKA and rewritten at launch whenever its content changes (edits are lost).
        # Requires python3 on PATH (Xcode Command Line Tools or Homebrew). If python3 is missing,
        # the hook cannot start and the agent simply carries on without MIKA.
        #
        # Contract (never block the agent):
        #   - always exits 0, whatever happens;
        #   - prints nothing unless MIKA returned an explicit allow / always / deny decision for
        #     a PermissionRequest (no output = the agent shows its own permission prompt);
        #   - every other event is fire-and-forget with a 0.3 s timeout.
        import os
        import sys

        SOCKET_PATH = \#(socketLiteral)
        MAX_BYTES = \#(maxRequestBytes)
        PERMISSION_TIMEOUT = 118
        AGENT = 'codex' if sys.argv[1:2] == ['--codex'] else 'claude'


        def _trim(value, limit=2000):
            # Non-permission events only need names and short labels: drop bulky strings.
            if isinstance(value, str):
                return value[:limit]
            if isinstance(value, dict):
                return {k: _trim(v, limit) for k, v in value.items()}
            if isinstance(value, list):
                return [_trim(v, limit) for v in value[:50]]
            return value


        def _send(data, timeout, wait_for_reply):
            import socket
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            try:
                s.settimeout(timeout)
                s.connect(SOCKET_PATH)
                s.sendall(data)
                if not wait_for_reply:
                    return b''
                chunks = []
                while True:
                    chunk = s.recv(4096)
                    if not chunk:
                        break
                    chunks.append(chunk)
                    if b'\n' in chunk:
                        break
                return b''.join(chunks)
            finally:
                s.close()


        def _permission_output(payload, reply):
            import json
            text = reply.decode('utf-8', 'replace').strip()
            if not text:
                return None
            decision = json.loads(text.splitlines()[0]).get('permissionDecision', '')
            if decision == 'allow':
                body = {'behavior': 'allow'}
            elif decision == 'always' and AGENT == 'codex':
                # Codex fails closed on updatedPermissions: a plain allow.
                body = {'behavior': 'allow'}
            elif decision == 'always':
                # Claude Code saves these rules; MIKA showed their text before the click.
                body = {'behavior': 'allow', 'updatedPermissions': payload.get('permission_suggestions') or []}
            elif decision == 'deny':
                body = {'behavior': 'deny', 'message': 'Denied from MIKA'}
            else:
                return None  # 'ask' or anything else: let Claude Code ask
            return json.dumps({'hookSpecificOutput': {'hookEventName': 'PermissionRequest', 'decision': body}})


        def main():
            import json
            raw = sys.stdin.buffer.read()
            if not raw:
                return None
            payload = json.loads(raw)
            if not isinstance(payload, dict):
                return None

            # Terminal context, so MIKA knows which app the session runs in.
            env = os.environ
            payload.setdefault('term_program', env.get('TERM_PROGRAM', ''))
            payload.setdefault('iterm_session_id', env.get('ITERM_SESSION_ID', ''))
            payload.setdefault('term_session_id', env.get('TERM_SESSION_ID', ''))
            payload.setdefault('bundle_id', env.get('__CFBundleIdentifier', ''))
            # Always overwritten: argv, not the payload, says which agent ran us.
            payload['_agent'] = AGENT
            if not payload.get('cwd'):
                payload['cwd'] = os.getcwd()

            if payload.get('hook_event_name') == 'PermissionRequest':
                # Blocks until MIKA answers (the app gives up after 115 s).
                data = (json.dumps(payload) + '\n').encode('utf-8')
                if len(data) > MAX_BYTES:
                    return None
                return _permission_output(payload, _send(data, PERMISSION_TIMEOUT, True))

            payload.pop('tool_response', None)
            data = (json.dumps(_trim(payload)) + '\n').encode('utf-8')
            if len(data) <= MAX_BYTES:
                _send(data, 0.3, False)
            return None


        try:
            output = main()
        except BaseException:
            output = None
        if output:
            try:
                sys.stdout.write(output + '\n')
                sys.stdout.flush()
            except BaseException:
                pass
        os._exit(0)

        """#
    }
}

// MARK: - Notification names for hook server → controller communication

extension Notification.Name {
    static let hookExpand = Notification.Name("mika.hookExpand")
}
