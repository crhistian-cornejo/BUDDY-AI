import Foundation

/// Size caps for files MIKA reads into memory (dropped files sent to Claude). Mail.app attaches a dropped file by
/// path, so emails have no cap of their own (only folders are refused).
enum FileLimits {
    /// Claude API image limit.
    static let maxImageBytes = 5 * 1024 * 1024
    static let maxPDFBytes = 10 * 1024 * 1024
    /// Text / code files are inlined in the prompt.
    static let maxTextBytes = 200_000
    /// Largest file copied into the drop inbox; bigger files are used in place.
    static let maxInboxCopyBytes = 100 * 1024 * 1024

    /// Size of a regular file (symlinks resolved), or nil for folders and unreadable items.
    static func regularFileSize(_ url: URL) -> Int? {
        let resolved = url.resolvingSymlinksInPath()
        guard let values = try? resolved.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey]),
              values.isRegularFile == true else { return nil }
        return values.fileSize ?? 0
    }

    /// Reads a regular file only if it is at most `maxBytes` long; never loads more than that.
    static func read(_ url: URL, maxBytes: Int) -> Data? {
        guard let size = regularFileSize(url), size <= maxBytes,
              let handle = try? FileHandle(forReadingFrom: url.resolvingSymlinksInPath()) else { return nil }
        defer { try? handle.close() }
        // Read one byte more than allowed so a file that grew since the size check is rejected.
        let data = (try? handle.read(upToCount: maxBytes + 1)) ?? Data()
        return data.count <= maxBytes ? data : nil
    }
}
