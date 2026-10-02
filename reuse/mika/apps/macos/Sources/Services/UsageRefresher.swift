import Foundation

extension Notification.Name {
    /// The plan figures changed (Limits): the usage bubble redraws.
    static let usageChanged = Notification.Name("mika.usageChanged")
}

/// Keeps the plan figures of Claude and Codex fresh without waiting for a turn, for the usage bubble beside the notch.
///  * Codex answers `account/rateLimits/read` on request: free, every 5 minutes.
///  * Claude only reports its windows while it answers: a one-word turn on the smallest model, every 30 minutes (about
///    half a cent of API value; the subscription does not notice). It is skipped when a figure from a real turn is recent.
/// Both are skipped when the client is not installed. Nothing here sends anything but those two requests.
@MainActor
final class UsageRefresher {
    static let shared = UsageRefresher()
    private var ticker: Task<Void, Never>?
    private var lastClaude = Date.distantPast

    static let claudeEvery: TimeInterval = 30 * 60
    static let codexEvery: TimeInterval = 5 * 60

    func start() {
        guard ticker == nil else { return }
        ticker = Task { [weak self] in
            try? await Task.sleep(for: .seconds(12))
            var codexAt = Date.distantPast
            while !Task.isCancelled {
                guard let self else { return }
                if Date().timeIntervalSince(codexAt) >= Self.codexEvery { codexAt = Date(); await self.refreshCodex() }
                if Date().timeIntervalSince(self.lastClaude) >= Self.claudeEvery { self.lastClaude = Date(); await self.refreshClaude() }
                try? await Task.sleep(for: .seconds(60))
            }
        }
    }

    // MARK: Codex

    func refreshCodex() async {
        _ = try? await readCodex()
    }

    func readCodex() async throws -> JSONValue {
        let result = try await codexRequest(method: "account/rateLimits/read", params: .object([:]))
        Limits.shared.recordCodex(params: result)
        NotificationCenter.default.post(name: .usageChanged, object: nil)
        return result
    }

    func readCodexActivity() async throws -> JSONValue {
        try await codexRequest(method: "account/usage/read", params: .object([:]))
    }

    /// Called only after the user confirms in Settings. Retries reuse the same idempotency key.
    func consumeCodexReset(creditID: String?, attemptID: String) async throws -> String {
        var params: [String: JSONValue] = ["idempotencyKey": .string(attemptID)]
        if let creditID { params["creditId"] = .string(creditID) }
        let result = try await codexRequest(method: "account/rateLimitResetCredit/consume", params: .object(params))
        guard let outcome = result["outcome"].stringValue else { throw UsageError.unavailable }
        return outcome
    }

    private enum UsageError: LocalizedError {
        case unavailable
        var errorDescription: String? { "No se pudo obtener la respuesta de Codex. Comprueba la conexión o actualiza Codex." }
    }

    private func codexRequest(method: String, params: JSONValue) async throws -> JSONValue {
        guard let codex = CLILocator.find("codex") else { throw UsageError.unavailable }
        let process = ProviderProcess(executable: codex, arguments: ["app-server", "--disable", "apps", "--disable", "shell_tool"],
                                      environment: ProviderEnvironment.scrubbed(ProcessInfo.processInfo.environment),
                                      currentDirectory: FileManager.default.temporaryDirectory)
        try process.start(stdin: nil, keepStdinOpen: true)
        let watchdog = Task { try? await Task.sleep(for: .seconds(30)); process.terminate() }
        defer { watchdog.cancel(); process.terminate() }
        process.writeLine(CodexRequests.initialize(id: 1, experimental: true))
        for await line in process.lines {
            guard let value = try? JSONDecoder().decode(JSONValue.self, from: Data(line.utf8)) else { continue }
            if value["id"].intValue == 1 {
                guard value["error"] == .null else { throw UsageError.unavailable }
                process.writeLine(CodexRequests.initialized)
                let request = JSONValue.object(["id": .number(2), "method": .string(method), "params": params])
                let data = try JSONEncoder().encode(request)
                process.writeLine(String(decoding: data, as: UTF8.self))
            } else if value["id"].intValue == 2 {
                guard value["error"] == .null, value["result"] != .null else { throw UsageError.unavailable }
                return value["result"]
            }
        }
        throw UsageError.unavailable
    }

    // MARK: Claude

    func refreshClaude() async {
        guard let claude = CLILocator.find("claude") else { return }
        let process = ProviderProcess(executable: claude,
                                      arguments: ["-p", "--restricted", "--output-format", "stream-json", "--verbose", "--permission-mode", "dontAsk",
                                                  "--tools", "", "--model", "claude-haiku-4-5-20251001"],
                                      environment: ProviderEnvironment.scrubbed(ProcessInfo.processInfo.environment),
                                      currentDirectory: FileManager.default.temporaryDirectory)
        do { try process.start(stdin: Data("responde: ok".utf8)) } catch { return }
        let watchdog = Task { try? await Task.sleep(nanoseconds: 60_000_000_000); process.terminate() }
        defer { watchdog.cancel(); process.terminate() }
        for await line in process.lines {
            guard let object = (try? JSONSerialization.jsonObject(with: Data(line.utf8))) as? [String: Any] else { continue }
            if object["type"] as? String == "rate_limit_event", let info = object["rate_limit_info"] as? [String: Any] {
                Limits.shared.recordClaude(info: info)
                NotificationCenter.default.post(name: .usageChanged, object: nil)
            }
            if object["type"] as? String == "result" { return }
        }
    }
}
