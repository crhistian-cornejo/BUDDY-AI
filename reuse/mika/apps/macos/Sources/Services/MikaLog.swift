import Foundation

/// Local diagnostic log: ~/Library/Logs/MIKA/mika.log.
///
/// Rules: never log secrets, commands, prompts or tool inputs in clear — only event and tool
/// names, sizes and short ids. `debug(_:)` lines (which may carry a redacted, truncated
/// excerpt) are written in DEBUG builds only. The file is capped at 512 KB with one rotated
/// copy (mika.log.1). Nothing ever leaves the Mac.
enum MikaLog {
    private static let queue = DispatchQueue(label: "io.github.crhistian-cornejo.mika.log", qos: .utility)
    private static let maxBytes: UInt64 = 512 * 1024

    /// Always written. Callers must only pass non-sensitive text.
    static func info(_ message: String) {
        write(message)
    }

    /// Written in DEBUG builds only.
    static func debug(_ message: @autoclosure () -> String) {
        #if DEBUG
        write(message())
        #endif
    }

    /// Truncated excerpt with anything credential-like masked. For DEBUG lines only.
    static func redacted(_ text: String, limit: Int = 40) -> String {
        var excerpt = String(text.prefix(4_000))
        let patterns = [
            // key=value / key: value / "Bearer xxx" style secrets
            #"(?i)(api[_-]?key|token|secret|passw(or)?d|authorization|bearer)\S*[=:\s]+\S+"#,
            // long opaque tokens (API keys, JWTs, hashes…)
            #"[A-Za-z0-9_\-\.]{20,}"#,
        ]
        for pattern in patterns {
            excerpt = excerpt.replacingOccurrences(of: pattern, with: "‹redacted›", options: .regularExpression)
        }
        let short = String(excerpt.prefix(limit))
        return short.count < excerpt.count || text.count > 4_000 ? short + "…" : short
    }

    private static func write(_ message: String) {
        let line = message.replacingOccurrences(of: "\n", with: " ")
        let date = Date()
        let limit = maxBytes
        queue.async {
            let fm = FileManager.default
            let dir = fm.urls(for: .libraryDirectory, in: .userDomainMask)[0]
                .appendingPathComponent("Logs/MIKA")
            try? fm.createDirectory(at: dir, withIntermediateDirectories: true,
                                    attributes: [.posixPermissions: NSNumber(value: 0o700)])
            let file = dir.appendingPathComponent("mika.log")

            // Simple rotation: mika.log → mika.log.1 once it passes the cap.
            if let size = (try? fm.attributesOfItem(atPath: file.path))?[.size] as? NSNumber,
               size.uint64Value > limit {
                let rotated = dir.appendingPathComponent("mika.log.1")
                try? fm.removeItem(at: rotated)
                try? fm.moveItem(at: file, to: rotated)
            }

            let formatter = DateFormatter()
            formatter.locale = Locale(identifier: "en_US_POSIX")
            formatter.dateFormat = "yyyy-MM-dd HH:mm:ss"
            guard let data = "\(formatter.string(from: date)) \(line)\n".data(using: .utf8) else { return }

            if let handle = try? FileHandle(forWritingTo: file) {
                defer { try? handle.close() }
                _ = try? handle.seekToEnd()
                try? handle.write(contentsOf: data)
            } else {
                _ = fm.createFile(atPath: file.path, contents: data,
                                  attributes: [.posixPermissions: NSNumber(value: 0o600)])
            }
        }
    }
}
