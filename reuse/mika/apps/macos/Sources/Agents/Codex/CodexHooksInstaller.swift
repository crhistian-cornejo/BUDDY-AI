import Foundation
import Darwin

/// Installs and removes MIKA's hooks in Codex's ~/.codex/hooks.json — the twin of
/// ClaudeHooksInstaller, with the same rules (it reuses that type's plan, diff, backup and
/// atomic write):
///  - a file that exists but is not a JSON object is never rewritten (clear error instead);
///  - `planInstall()` / `planUninstall()` only compute the new content and a line diff;
///    `apply(_:)` writes only after the user confirmed that diff, re-reads the file and
///    aborts if it changed since the preview, and takes a backup that must succeed;
///  - only entries whose command is exactly MIKA's relay in Codex mode are added or removed.
///
/// Codex specifics (https://developers.openai.com/codex/hooks):
///  - user hooks live in `$CODEX_HOME/hooks.json` (default ~/.codex), or inline in
///    config.toml. MIKA writes the JSON file so it never rewrites the user's TOML;
///  - the file is `{ "description"?, "hooks": { "<Event>": [group…] } }` and Codex rejects
///    any other top-level key, so MIKA only ever touches "hooks";
///  - Codex runs the command with `$SHELL -lc`, so it is the quoted relay path + `--codex`;
///  - Codex skips a new or changed hook until the user trusts it in Codex's `/hooks`
///    (a hash Codex keeps in config.toml). MIKA never writes that trust: the settings
///    window tells the user to do it;
///  - `timeout` is in seconds; SessionEnd and Interrupt allow at most 3.
enum CodexHooksInstaller {

    typealias Action = ClaudeHooksInstaller.Action
    typealias Plan = ClaudeHooksInstaller.Plan
    typealias DiffLine = ClaudeHooksInstaller.DiffLine

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
                return "Could not read Codex's hooks.json (\(reason)). Nothing was changed."
            case .invalidJSON:
                return "Codex's hooks.json is not valid JSON. Fix or move it, then try again. Nothing was changed."
            case .unexpectedStructure(let detail):
                return "Codex's hooks.json has an unexpected shape (\(detail)). Nothing was changed."
            case .changedSincePreview:
                return "Codex's hooks.json changed since the preview. Nothing was written — review the changes again."
            case .backupFailed(let reason):
                return "Could not back up Codex's hooks.json (\(reason)). Nothing was written."
            case .writeFailed(let reason):
                return "Could not write Codex's hooks.json (\(reason)). The previous file is untouched."
            }
        }
    }

    /// Codex hook events MIKA listens to, with their timeout in seconds. PermissionRequest
    /// outwaits the relay (118 s). Codex has no Notification / StopFailure /
    /// PostToolUseFailure; it has Interrupt.
    static let events: [(name: String, timeout: Int)] = [
        ("SessionStart", 10), ("SessionEnd", 3),
        ("UserPromptSubmit", 10),
        ("PreToolUse", 10), ("PostToolUse", 10),
        ("PermissionRequest", 120),
        ("Stop", 10),
        ("SubagentStart", 10), ("SubagentStop", 10),
        ("Interrupt", 3),
    ]

    /// Shown by Codex while the PermissionRequest hook runs (the card is up in the island).
    static let waitingMessage = "Waiting for your answer in MIKA"

    /// `$CODEX_HOME` when MIKA sees it (an app opened from Finder does not inherit shell
    /// variables), else ~/.codex — where Codex itself looks.
    static var codexHome: URL {
        if let dir = ProcessInfo.processInfo.environment["CODEX_HOME"], !dir.isEmpty {
            return URL(fileURLWithPath: (dir as NSString).expandingTildeInPath, isDirectory: true)
        }
        return FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".codex", isDirectory: true)
    }

    static var hooksURL: URL { codexHome.appendingPathComponent("hooks.json") }

    /// The exact command MIKA registers: the relay's path quoted for the shell, then `--codex`.
    static var hookCommand: String { ClaudeHooksInstaller.hookCommand + " --codex" }

    /// True only for MIKA's own relay in Codex mode (exact match — never a substring match).
    static func isOurCommand(_ command: String) -> Bool {
        let trimmed = command.trimmingCharacters(in: .whitespaces)
        return trimmed == hookCommand || trimmed == HookServer.hookScriptPath + " --codex"
    }

    // MARK: - Status (read-only)

    /// True when MIKA's relay is registered for SessionStart in hooks.json.
    static func isInstalled() -> Bool {
        guard let data = try? Data(contentsOf: hooksURL),
              let file = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              let hooks = file["hooks"] as? [String: Any],
              let groups = hooks["SessionStart"] as? [Any] else { return false }
        for case let group as [String: Any] in groups {
            for case let entry as [String: Any] in ((group["hooks"] as? [Any]) ?? []) {
                if let command = entry["command"] as? String, isOurCommand(command) { return true }
            }
        }
        return false
    }

    // MARK: - Preview

    static func planInstall() throws -> Plan { try makePlan(.install) }
    static func planUninstall() throws -> Plan { try makePlan(.uninstall) }

    private static func makePlan(_ action: Action) throws -> Plan {
        let target = hooksURL.resolvingSymlinksInPath()
        let original = try shared { try ClaudeHooksInstaller.readIfExists(target) }

        var file: [String: Any] = [:]
        var oldText = ""
        if let original {
            guard let object = try? JSONSerialization.jsonObject(with: original),
                  let dict = object as? [String: Any] else { throw InstallerError.invalidJSON }
            file = dict
            oldText = try shared { try ClaudeHooksInstaller.serializedText(dict) }
        }

        let hadHooksKey = file["hooks"] != nil
        var hooks: [String: Any] = [:]
        if let existing = file["hooks"] {
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

        // 2. Install: append one group per event (last, so other groups keep their index —
        //    Codex keys its trust on it). No matcher: every occurrence counts.
        if action == .install {
            for (event, timeout) in events {
                var groups: [Any] = []
                if let existing = hooks[event] {
                    guard let array = existing as? [Any] else {
                        throw InstallerError.unexpectedStructure("hooks.\(event) is not a list")
                    }
                    groups = array
                }
                var entry: [String: Any] = ["type": "command", "command": hookCommand, "timeout": timeout]
                if event == "PermissionRequest" { entry["statusMessage"] = waitingMessage }
                groups.append(["hooks": [entry]] as [String: Any])
                hooks[event] = groups
            }
        }

        if !hooks.isEmpty {
            file["hooks"] = hooks
        } else if hadHooksKey && removedAny {
            file.removeValue(forKey: "hooks")   // MIKA's entries were all it held
        }

        let newText = try shared { try ClaudeHooksInstaller.serializedText(file) }
        return Plan(action: action,
                    targetURL: target,
                    originalData: original,
                    updatedData: Data(newText.utf8),
                    diff: ClaudeHooksInstaller.lineDiff(old: oldText, new: newText))
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
        let target = hooksURL.resolvingSymlinksInPath()
        guard target == plan.targetURL else { throw InstallerError.changedSincePreview }
        let current = try shared { try ClaudeHooksInstaller.readIfExists(target) }
        guard current == plan.originalData else { throw InstallerError.changedSincePreview }

        var backupURL: URL? = nil
        var mode: mode_t = 0o644
        if let current {
            backupURL = try shared { try ClaudeHooksInstaller.writeBackup(current, beside: hooksURL) }
            mode = ClaudeHooksInstaller.fileMode(of: target) ?? mode
        } else {
            do {
                try FileManager.default.createDirectory(at: target.deletingLastPathComponent(),
                                                        withIntermediateDirectories: true)
            } catch {
                throw InstallerError.writeFailed(error.localizedDescription)
            }
        }
        try shared { try ClaudeHooksInstaller.atomicWrite(plan.updatedData, to: target, mode: mode) }
        return backupURL
    }

    /// Runs one of ClaudeHooksInstaller's file helpers and reports its failure as being
    /// about hooks.json rather than ~/.claude/settings.json.
    private static func shared<T>(_ body: () throws -> T) throws -> T {
        do {
            return try body()
        } catch let error as ClaudeHooksInstaller.InstallerError {
            switch error {
            case .unreadable(let reason):          throw InstallerError.unreadable(reason)
            case .invalidJSON:                     throw InstallerError.invalidJSON
            case .unexpectedStructure(let detail): throw InstallerError.unexpectedStructure(detail)
            case .changedSincePreview:             throw InstallerError.changedSincePreview
            case .backupFailed(let reason):        throw InstallerError.backupFailed(reason)
            case .writeFailed(let reason):         throw InstallerError.writeFailed(reason)
            }
        }
    }
}
