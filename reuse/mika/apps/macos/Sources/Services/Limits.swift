import Foundation

/// How much of the Claude and Codex plans is used (the 5-hour and weekly windows), learnt for free from what the
/// providers already say during MIKA's own turns: Claude's `rate_limit_event` and Codex's `account/rateLimits/updated`.
/// Nothing is asked for on purpose, so a provider shows up here after its first turn in MIKA, and a figure carries the
/// time it was seen. Twin of apps/windows/src-tauri/src/services/limits.rs; the figures are kept in
/// `~/Library/Application Support/Mika/usage-limits.json`.
final class Limits: @unchecked Sendable {
    struct Window: Codable, Equatable, Sendable {
        /// "5 h", "semana"…
        var label: String
        /// 0–100.
        var usedPct: Double
        /// Unix seconds when the window resets, if the provider said.
        var resetsAt: Int64?
        /// Unix milliseconds when this was seen.
        var seenMs: Int64
    }

    static let shared = Limits(file: FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("Mika/usage-limits.json"))

    private let file: URL
    private let lock = NSLock()
    private var loaded: [String: [Window]]?

    init(file: URL) { self.file = file }

    // MARK: - Reading what the providers say

    /// Claude Code: `{"type":"rate_limit_event","rate_limit_info":{"rateLimitType":"five_hour","utilization":0.42,"resetsAt":…}}`;
    /// `info` is the `rate_limit_info` object.
    func recordClaude(info: [String: Any], nowMs: Int64 = Limits.nowMs()) {
        if let unified = info["unifiedWindows"] as? [String: Any] {
            for (key, value) in unified {
                guard let window = value as? [String: Any] else { continue }
                recordClaude(info: window.merging(["rateLimitType": key]) { _, new in new }, nowMs: nowMs)
            }
        }
        guard let n = Self.number(info["utilization"]), n.isFinite,
              let type = info["rateLimitType"] as? String else { return }
        let label: String
        switch type {
        case "five_hour": label = "5 h"
        case "seven_day": label = "semana"
        case "seven_day_sonnet": label = "semana · Sonnet"
        case "seven_day_opus": label = "semana · Opus"
        case "monthly", "thirty_day": label = "mes"
        default: label = type.replacingOccurrences(of: "_", with: " ")
        }
        store("claude", Window(label: label, usedPct: Self.percent(n),
                               resetsAt: Self.number(info["resetsAt"]).flatMap(Self.seconds), seenMs: nowMs))
    }

    /// Keep every metered bucket, including model-specific and monthly windows.
    func recordCodex(params: JSONValue, nowMs: Int64 = Limits.nowMs()) {
        var buckets: [String: JSONValue] = [:]
        if case .object(let byID) = params["rateLimitsByLimitId"], !byID.isEmpty { buckets = byID }
        else if params["rateLimits"] != .null { buckets = ["codex": params["rateLimits"]] }
        var windows: [Window] = []
        for (id, bucket) in buckets.sorted(by: { $0.key < $1.key }) {
            for key in ["primary", "secondary"] {
                let window = bucket[key]
                guard let used = Self.jsonNumber(window["usedPercent"]), used.isFinite else { continue }
                let period = Self.jsonNumber(window["windowDurationMins"]).flatMap { Int64(exactly: $0.rounded()) }.map(Self.minutesLabel)
                    ?? (key == "primary" ? "principal" : "secundaria")
                let label = id == "codex" ? period : "\(bucket["limitName"].stringValue ?? id) · \(period)"
                windows.append(Window(label: label, usedPct: min(max(used, 0), 100),
                                      resetsAt: Self.jsonNumber(window["resetsAt"]).flatMap(Self.seconds), seenMs: nowMs))
            }
        }
        // A read is authoritative; removed or null windows must not survive from an older account/plan.
        if !buckets.isEmpty {
            lock.withLock {
                var all = current()
                all["codex"] = windows
                loaded = all
                if let data = try? JSONEncoder().encode(all) {
                    try? FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
                    try? data.write(to: file, options: .atomic)
                }
            }
        }
    }

    /// Provider ("claude", "codex") → its windows.
    func snapshot() -> [String: [Window]] { lock.withLock { current() } }

    // MARK: - Pure helpers

    /// Claude sends a fraction (0.42); Codex a percentage (42).
    static func percent(_ n: Double) -> Double {
        guard n.isFinite else { return 0 }
        return min(max(n <= 1 ? n * 100 : n, 0), 100)
    }

    /// Unix seconds, from seconds or milliseconds.
    static func seconds(_ n: Double) -> Int64? {
        guard n.isFinite, abs(n) < 1e15 else { return nil }
        let value = Int64(n)
        return value > 10_000_000_000 ? value / 1000 : value
    }

    static func minutesLabel(_ mins: Int64) -> String {
        switch mins {
        case 0...359: return "\(max(Int((Double(mins) / 60).rounded()), 1)) h"
        case 360...2880: return "día"
        case 2881...10080: return mins == 10080 ? "semana" : "\(mins / 1440) días"
        case 10081...44640: return mins >= 40320 ? "mes" : "\(mins / 1440) días"
        default: return "\(mins / 1440) días"
        }
    }

    static func nowMs() -> Int64 { Int64(Date().timeIntervalSince1970 * 1000) }

    private static func number(_ value: Any?) -> Double? {
        if let n = value as? NSNumber, CFGetTypeID(n) != CFBooleanGetTypeID() { return n.doubleValue }
        return nil
    }

    private static func jsonNumber(_ value: JSONValue) -> Double? {
        if case .number(let n) = value { return n }
        return nil
    }

    // MARK: - Storage

    /// Must be called with the lock held.
    private func current() -> [String: [Window]] {
        if let loaded { return loaded }
        let read = (try? Data(contentsOf: file)).flatMap { try? JSONDecoder().decode([String: [Window]].self, from: $0) } ?? [:]
        loaded = read
        return read
    }

    private func store(_ provider: String, _ window: Window) {
        lock.withLock {
            var all = current()
            var list = all[provider] ?? []
            if let i = list.firstIndex(where: { $0.label == window.label }) { list[i] = window } else { list.append(window) }
            all[provider] = list
            loaded = all
            guard let data = try? JSONEncoder().encode(all) else { return }
            try? FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true,
                                                     attributes: [.posixPermissions: 0o700])
            try? data.write(to: file, options: .atomic)
        }
    }
}
