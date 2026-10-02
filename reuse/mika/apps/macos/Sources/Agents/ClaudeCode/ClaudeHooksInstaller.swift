import Foundation
import Darwin

/// Installs and removes MIKA's hooks in ~/.claude/settings.json.
///
/// Claude Code's settings are never clobbered:
///  - a file that exists but is not a JSON object is never rewritten (clear error instead);
///  - `planInstall()` / `planUninstall()` compute the new content and a line diff without
///    writing anything; `apply(_:)` writes only after the user confirmed that diff;
///  - `apply(_:)` re-reads the file and aborts if it changed since the preview (for instance
///    Claude Code saved an "Always allow" rule meanwhile), takes a backup that must succeed,
///    then writes atomically (temp file + rename), keeping the file mode and any symlink;
///  - only entries whose command is exactly MIKA's relay are added or removed; other tools'
///    hooks, even in the same matcher group, are left untouched.
enum ClaudeHooksInstaller {

    enum Action: Sendable {
        case install, uninstall
    }

    struct DiffLine: Sendable, Identifiable, Hashable {
        enum Kind: Sendable, Hashable { case same, added, removed, gap }
        let id: Int
        let kind: Kind
        let text: String
    }

    struct Plan: Sendable {
        let action: Action
        /// The file that will be written (symlinks resolved).
        let targetURL: URL
        /// Bytes read for the preview (nil = the file did not exist). Used as the fingerprint.
        let originalData: Data?
        let updatedData: Data
        /// Changed lines with a little context (keys sorted, as MIKA will write them).
        let diff: [DiffLine]

        var hasChanges: Bool { !diff.isEmpty }
    }

    enum InstallerError: LocalizedError {
        case unreadable(String)
        case invalidJSON
        case unexpectedStructure(String)
        case changedSincePreview
        case backupFailed(String)
        case writeFailed(String)

        var errorDescription: String? {
            switch self {
            case .unreadable(let reason):
                return "Could not read ~/.claude/settings.json (\(reason)). Nothing was changed."
            case .invalidJSON:
                return "~/.claude/settings.json is not valid JSON. Fix or move it, then try again. Nothing was changed."
            case .unexpectedStructure(let detail):
                return "~/.claude/settings.json has an unexpected shape (\(detail)). Nothing was changed."
            case .changedSincePreview:
                return "~/.claude/settings.json changed since the preview. Nothing was written — review the changes again."
            case .backupFailed(let reason):
                return "Could not back up ~/.claude/settings.json (\(reason)). Nothing was written."
            case .writeFailed(let reason):
                return "Could not write ~/.claude/settings.json (\(reason)). The previous file is untouched."
            }
        }
    }

    /// Hook events MIKA listens to, with their timeout in seconds.
    static let events: [(name: String, timeout: Int)] = [
        ("SessionStart", 10), ("SessionEnd", 10),
        ("UserPromptSubmit", 10),
        ("PreToolUse", 10), ("PostToolUse", 10), ("PostToolUseFailure", 10),
        ("PermissionRequest", 120),
        ("Notification", 10),
        ("Stop", 10), ("StopFailure", 10),
        ("SubagentStart", 10), ("SubagentStop", 10),
    ]

    static var settingsURL: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".claude/settings.json")
    }

    /// The exact command MIKA registers: the relay's path, quoted for the shell.
    static var hookCommand: String {
        "\"" + HookServer.hookScriptPath.replacingOccurrences(of: "\"", with: "\\\"") + "\""
    }

    /// True only for MIKA's own relay (exact path match — never a substring match).
    static func isOurCommand(_ command: String) -> Bool {
        let trimmed = command.trimmingCharacters(in: .whitespaces)
        return trimmed == hookCommand || trimmed == HookServer.hookScriptPath
    }

    // MARK: - Status (read-only)

    /// True when MIKA's relay is registered for SessionStart.
    static func isInstalled() -> Bool {
        guard let hooks = currentHooks() else { return false }
        return ourTimeouts(in: hooks)["SessionStart"] != nil
    }

    /// True when MIKA's PermissionRequest hook times out before 120 s (approvals would fail).
    static func needsUpdate() -> Bool {
        guard let hooks = currentHooks(), let timeout = ourTimeouts(in: hooks)["PermissionRequest"] else {
            return false
        }
        return timeout < 120
    }

    private static func currentHooks() -> [String: Any]? {
        guard let data = try? Data(contentsOf: settingsURL),
              let settings = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { return nil }
        return settings["hooks"] as? [String: Any]
    }

    /// Timeout of MIKA's entry for each event (Claude Code's default of 60 s when unset).
    private static func ourTimeouts(in hooks: [String: Any]) -> [String: Int] {
        var result: [String: Int] = [:]
        for (event, value) in hooks {
            guard let groups = value as? [Any] else { continue }
            for case let group as [String: Any] in groups {
                for case let entry as [String: Any] in ((group["hooks"] as? [Any]) ?? []) {
                    if let command = entry["command"] as? String, isOurCommand(command) {
                        result[event] = (entry["timeout"] as? Int) ?? 60
                    }
                }
            }
        }
        return result
    }

    // MARK: - Preview

    static func planInstall() throws -> Plan { try makePlan(.install) }
    static func planUninstall() throws -> Plan { try makePlan(.uninstall) }

    private static func makePlan(_ action: Action) throws -> Plan {
        let target = settingsURL.resolvingSymlinksInPath()
        let original = try readIfExists(target)

        var settings: [String: Any] = [:]
        var oldText = ""
        if let original {
            guard let object = try? JSONSerialization.jsonObject(with: original),
                  let dict = object as? [String: Any] else { throw InstallerError.invalidJSON }
            settings = dict
            oldText = try serializedText(dict)
        }

        let hadHooksKey = settings["hooks"] != nil
        var hooks: [String: Any] = [:]
        if let existing = settings["hooks"] {
            guard let dict = existing as? [String: Any] else {
                throw InstallerError.unexpectedStructure("\"hooks\" is not an object")
            }
            hooks = dict
        }

        // 1. Remove MIKA's own entries everywhere (install adds fresh ones below).
        var removedAny = false
        for (event, value) in hooks {
            guard let groups = value as? [Any] else { continue }   // unknown shape: leave it alone
            let cleaned = removingOurEntries(from: groups)
            guard cleaned.removed else { continue }
            removedAny = true
            if cleaned.groups.isEmpty {
                hooks.removeValue(forKey: event)
            } else {
                hooks[event] = cleaned.groups
            }
        }

        // 2. Install: append one group per event that runs the relay.
        if action == .install {
            for (event, timeout) in events {
                var groups: [Any] = []
                if let existing = hooks[event] {
                    guard let array = existing as? [Any] else {
                        throw InstallerError.unexpectedStructure("hooks.\(event) is not a list")
                    }
                    groups = array
                }
                let entry: [String: Any] = ["type": "command", "command": hookCommand, "timeout": timeout]
                let group: [String: Any] = ["hooks": [entry]]
                groups.append(group)
                hooks[event] = groups
            }
        }

        if !hooks.isEmpty {
            settings["hooks"] = hooks
        } else if hadHooksKey && removedAny {
            settings.removeValue(forKey: "hooks")   // MIKA's entries were all it held
        }

        let newText = try serializedText(settings)
        return Plan(action: action,
                    targetURL: target,
                    originalData: original,
                    updatedData: Data(newText.utf8),
                    diff: lineDiff(old: oldText, new: newText))
    }

    /// Drops MIKA's entries from one event's matcher groups. A group disappears only when
    /// MIKA's entries were all it contained; other entries and keys are kept as they are.
    private static func removingOurEntries(from groups: [Any]) -> (groups: [Any], removed: Bool) {
        var removed = false
        var result: [Any] = []
        for item in groups {
            guard var group = item as? [String: Any], let entries = group["hooks"] as? [Any] else {
                result.append(item)
                continue
            }
            let kept = entries.filter { entry in
                guard let dict = entry as? [String: Any], let command = dict["command"] as? String else {
                    return true
                }
                return !isOurCommand(command)
            }
            if kept.count == entries.count {
                result.append(item)
                continue
            }
            removed = true
            if kept.isEmpty { continue }
            group["hooks"] = kept
            result.append(group)
        }
        return (result, removed)
    }

    // MARK: - Apply

    /// Writes a confirmed plan. Returns the backup's URL (nil when there was no file to back up).
    @discardableResult
    static func apply(_ plan: Plan) throws -> URL? {
        let target = settingsURL.resolvingSymlinksInPath()
        guard target == plan.targetURL else { throw InstallerError.changedSincePreview }
        let current = try readIfExists(target)
        guard current == plan.originalData else { throw InstallerError.changedSincePreview }

        var backupURL: URL? = nil
        var mode: mode_t = 0o644
        if let current {
            backupURL = try writeBackup(current)
            mode = fileMode(of: target) ?? mode
        } else {
            do {
                try FileManager.default.createDirectory(at: target.deletingLastPathComponent(),
                                                        withIntermediateDirectories: true)
            } catch {
                throw InstallerError.writeFailed(error.localizedDescription)
            }
        }
        try atomicWrite(plan.updatedData, to: target, mode: mode)
        return backupURL
    }

    // MARK: - File helpers (also used by CodexHooksInstaller for ~/.codex/hooks.json)

    static func readIfExists(_ url: URL) throws -> Data? {
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        do {
            return try Data(contentsOf: url)
        } catch {
            throw InstallerError.unreadable(error.localizedDescription)
        }
    }

    /// <file>.bak-YYYYMMDD-HHMMSS-<random> (settings.json.bak-… by default), next to the file, mode 0600.
    static func writeBackup(_ data: Data, beside file: URL = settingsURL) throws -> URL {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "yyyyMMdd-HHmmss"
        let suffix = String(UUID().uuidString.prefix(8)).lowercased()
        let url = file.deletingLastPathComponent()
            .appendingPathComponent("\(file.lastPathComponent).bak-\(formatter.string(from: Date()))-\(suffix)")
        do {
            try writeNewFile(data, to: url, mode: 0o600)
        } catch {
            throw InstallerError.backupFailed(error.localizedDescription)
        }
        return url
    }

    /// Temp file in the same directory, flushed, then rename(2) over the target.
    static func atomicWrite(_ data: Data, to target: URL, mode: mode_t) throws {
        let temp = target.deletingLastPathComponent()
            .appendingPathComponent(".\(target.lastPathComponent).mika-\(UUID().uuidString)")
        try writeNewFile(data, to: temp, mode: mode)
        guard rename(temp.path, target.path) == 0 else {
            let code = errno
            unlink(temp.path)
            throw InstallerError.writeFailed(String(cString: strerror(code)))
        }
    }

    /// Creates `url` (fails if it already exists), writes all of `data` and fsyncs it.
    private static func writeNewFile(_ data: Data, to url: URL, mode: mode_t) throws {
        let fd = open(url.path, O_WRONLY | O_CREAT | O_EXCL, mode)
        guard fd >= 0 else { throw InstallerError.writeFailed(String(cString: strerror(errno))) }
        var failure: Int32 = 0
        data.withUnsafeBytes { (buffer: UnsafeRawBufferPointer) in
            var offset = 0
            while offset < buffer.count {
                let n = write(fd, buffer.baseAddress! + offset, buffer.count - offset)
                if n <= 0 { failure = errno; return }
                offset += n
            }
        }
        if failure == 0 && fsync(fd) != 0 { failure = errno }
        close(fd)
        guard failure == 0 else {
            unlink(url.path)
            throw InstallerError.writeFailed(String(cString: strerror(failure)))
        }
        chmod(url.path, mode)   // open() applied the umask
    }

    static func fileMode(of url: URL) -> mode_t? {
        guard let attributes = try? FileManager.default.attributesOfItem(atPath: url.path),
              let permissions = attributes[.posixPermissions] as? NSNumber else { return nil }
        return mode_t(permissions.uint16Value & 0o777)
    }

    // MARK: - Serialisation and diff

    /// Pretty JSON with sorted keys and readable slashes: the exact text MIKA writes.
    static func serializedText(_ object: [String: Any]) throws -> String {
        guard JSONSerialization.isValidJSONObject(object) else {
            throw InstallerError.unexpectedStructure("not serialisable")
        }
        let data: Data
        do {
            data = try JSONSerialization.data(withJSONObject: object,
                                              options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
        } catch {
            throw InstallerError.unexpectedStructure(error.localizedDescription)
        }
        return String(decoding: data, as: UTF8.self)
    }

    /// Line diff (LCS) between the current and the new text, unchanged runs folded to
    /// `context` lines around each change. Empty when nothing changes.
    static func lineDiff(old: String, new: String, context: Int = 3) -> [DiffLine] {
        let a = old.isEmpty ? [] : old.components(separatedBy: "\n")
        let b = new.isEmpty ? [] : new.components(separatedBy: "\n")

        // Common prefix / suffix first, so the LCS table only covers the changed middle.
        var prefix = 0
        while prefix < a.count && prefix < b.count && a[prefix] == b[prefix] { prefix += 1 }
        var suffix = 0
        while suffix < a.count - prefix && suffix < b.count - prefix
                && a[a.count - 1 - suffix] == b[b.count - 1 - suffix] { suffix += 1 }
        let aMid = Array(a[prefix..<(a.count - suffix)])
        let bMid = Array(b[prefix..<(b.count - suffix)])

        var ops: [(kind: DiffLine.Kind, text: String)] = []
        for line in a[0..<prefix] { ops.append((kind: .same, text: line)) }

        let n = aMid.count
        let m = bMid.count
        if n * m <= 1_000_000 {
            var lcs = [[Int]](repeating: [Int](repeating: 0, count: m + 1), count: n + 1)
            for i in stride(from: n - 1, through: 0, by: -1) {
                for j in stride(from: m - 1, through: 0, by: -1) {
                    lcs[i][j] = aMid[i] == bMid[j] ? lcs[i + 1][j + 1] + 1 : max(lcs[i + 1][j], lcs[i][j + 1])
                }
            }
            var i = 0
            var j = 0
            while i < n && j < m {
                if aMid[i] == bMid[j] {
                    ops.append((kind: .same, text: aMid[i])); i += 1; j += 1
                } else if lcs[i + 1][j] >= lcs[i][j + 1] {
                    ops.append((kind: .removed, text: aMid[i])); i += 1
                } else {
                    ops.append((kind: .added, text: bMid[j])); j += 1
                }
            }
            while i < n { ops.append((kind: .removed, text: aMid[i])); i += 1 }
            while j < m { ops.append((kind: .added, text: bMid[j])); j += 1 }
        } else {
            for line in aMid { ops.append((kind: .removed, text: line)) }
            for line in bMid { ops.append((kind: .added, text: line)) }
        }
        for line in a[(a.count - suffix)...] { ops.append((kind: .same, text: line)) }

        // Keep changes plus `context` lines around them; fold the rest into "…".
        var keep = [Bool](repeating: false, count: ops.count)
        var hasChange = false
        for (index, op) in ops.enumerated() where op.kind != .same {
            hasChange = true
            for k in max(0, index - context)...min(ops.count - 1, index + context) { keep[k] = true }
        }
        guard hasChange else { return [] }

        var result: [DiffLine] = []
        var folded = false
        for (index, op) in ops.enumerated() {
            if keep[index] {
                result.append(DiffLine(id: result.count, kind: op.kind, text: op.text))
                folded = false
            } else if !folded {
                result.append(DiffLine(id: result.count, kind: .gap, text: "…"))
                folded = true
            }
        }
        return result
    }
}
