// Ported from MIKA (MIT, © MIKA contributors, revision d050bc5): apps/macos/Sources/Services/SourceIcons.swift
import Foundation

/// Site icons for the sources row under an answer: the favicon of each page an agent used. Twin of
/// apps/windows/src-tauri/src/services/icons.rs. Fetched from the site itself (never through a third-party favicon
/// service, which would learn every site the user reads about) with URLSession (the system's trust store), https only,
/// public hosts only (redirects too), at most 100 KB, checked by its real signature, and cached for a month in
/// `MIKA/icons/`. A site that answered without an icon is asked again after a week; no connection is not an answer.
/// Off with "Íconos de fuentes" in Settings: the row keeps its letter tiles and no site is contacted.
///
/// macOS draws raster icons only (PNG, ICO, GIF, JPEG, WebP): `/favicon.svg` is not asked for, so no untrusted SVG is
/// ever parsed.
enum SourceIcons {
    /// UserDefaults key of the "Íconos de fuentes" switch (on by default).
    static let settingKey = "sourceIcons"
    static let maxBytes = 100_000
    static let keep: TimeInterval = 30 * 24 * 3600
    static let retry: TimeInterval = 7 * 24 * 3600
    static let paths = ["/favicon.ico", "/apple-touch-icon.png"]

    /// Whether `source` may ask its site for its icon: only a site the core reported for this answer (one the
    /// agent's own tools searched or opened). A link that is only written in the answer's text never causes a
    /// request: its address is whatever the model wrote, and a request to it could carry data out with no click.
    static func mayFetch(_ source: ChatSource, reported: [ChatSource]) -> Bool {
        reported.contains { $0.host == source.host }
    }

    static var enabled: Bool { UserDefaults.standard.object(forKey: settingKey) as? Bool ?? true }

    /// `Buddy/icons/` in Application Support.
    static func cacheDir() -> URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Buddy/icons", isDirectory: true)
    }

    /// A host worth asking: a public name, not an address on the user's machine or network.
    static func publicHost(_ url: String) -> String? {
        guard url.hasPrefix("https://") || url.hasPrefix("http://"), let host = WebHost.of(url) else { return nil }
        let allowed = host.utf8.allSatisfy { (48...57).contains($0) || (97...122).contains($0) || $0 == 0x2E || $0 == 0x2D }
        let ok = host.utf8.count <= 253 && host.contains(".") && !host.hasPrefix(".") && !host.hasSuffix(".") && allowed
            // An IP address (only digits and dots) or a local name never leaves for the network.
            && !host.utf8.allSatisfy({ (48...57).contains($0) || $0 == 0x2E })
            && ![".local", ".localhost", ".internal", ".lan", ".home", ".corp", ".intranet"].contains { host.hasSuffix($0) }
        return ok ? host : nil
    }

    /// What the bytes really are, whatever the server said. Only the kinds macOS draws here (no SVG).
    static func sniff(_ bytes: Data) -> String? {
        let b = [UInt8](bytes.prefix(12))
        if b.starts(with: [0x89, 0x50, 0x4E, 0x47]) { return "image/png" }
        if b.starts(with: [0, 0, 1, 0]) { return "image/x-icon" }
        if b.starts(with: [0x47, 0x49, 0x46, 0x38]) { return "image/gif" }
        if b.starts(with: [0xFF, 0xD8, 0xFF]) { return "image/jpeg" }
        if b.count >= 12, b.starts(with: [0x52, 0x49, 0x46, 0x46]), Array(b[8..<12]) == [0x57, 0x45, 0x42, 0x50] { return "image/webp" }
        return nil
    }

    private static func cacheFile(_ host: String, in dir: URL) -> URL { dir.appendingPathComponent("\(host).bin") }
    private static func missFile(_ host: String, in dir: URL) -> URL { dir.appendingPathComponent("\(host).none") }

    private static func fresh(_ url: URL, maxAge: TimeInterval) -> Bool {
        guard let modified = (try? url.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate else { return false }
        return Date().timeIntervalSince(modified) < maxAge
    }

    /// The icon's bytes for the site behind `url`, or nil (the row keeps the letter tile).
    static func icon(for url: String, cacheDir: URL = SourceIcons.cacheDir()) async -> Data? {
        guard enabled, let host = publicHost(url) else { return nil }
        let cached = cacheFile(host, in: cacheDir)
        if fresh(cached, maxAge: keep), let bytes = try? Data(contentsOf: cached), sniff(bytes) != nil { return bytes }
        if fresh(missFile(host, in: cacheDir), maxAge: retry) { return nil }
        var answered = false
        for path in paths {
            guard let target = URL(string: "https://\(host)\(path)") else { continue }
            switch await download(target) {
            case .noAnswer:
                continue
            case .answered(let bytes):
                answered = true
                guard let bytes, bytes.count >= 16, bytes.count <= maxBytes, sniff(bytes) != nil else { continue }
                try? FileManager.default.createDirectory(at: cacheDir, withIntermediateDirectories: true,
                                                         attributes: [.posixPermissions: 0o700])
                try? bytes.write(to: cached, options: .atomic)
                return bytes
            }
        }
        // Only a site that answered without an icon is remembered as having none.
        if answered {
            try? FileManager.default.createDirectory(at: cacheDir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            try? Data().write(to: missFile(host, in: cacheDir), options: .atomic)
        }
        return nil
    }

    private enum Download { case noAnswer, answered(Data?) }

    private static let session: URLSession = {
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 5
        config.timeoutIntervalForResource = 8
        config.httpAdditionalHeaders = ["User-Agent": "MIKA"]
        return URLSession(configuration: config)
    }()

    /// Redirects only to another public https site, three at most: never into the user's own network.
    private final class PublicRedirects: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
        private let lock = NSLock()
        private var count = 0

        func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                        newRequest request: URLRequest) async -> URLRequest? {
            let hops = lock.withLock { () -> Int in count += 1; return count }
            guard hops <= 3, let next = request.url?.absoluteString, next.hasPrefix("https://"), SourceIcons.publicHost(next) != nil else { return nil }
            return request
        }
    }

    /// The body (at most `maxBytes` + 1 bytes are read) of a successful answer, nil for any other answer.
    private static func download(_ url: URL) async -> Download {
        var request = URLRequest(url: url, timeoutInterval: 5)
        request.setValue("image/*", forHTTPHeaderField: "Accept")
        let reply: (URLSession.AsyncBytes, URLResponse)
        do {
            reply = try await session.bytes(for: request, delegate: PublicRedirects())
        } catch {
            return .noAnswer
        }
        let (bytes, response) = reply
        guard let http = response as? HTTPURLResponse, http.url?.scheme?.lowercased() == "https" else { return .answered(nil) }
        guard (200..<300).contains(http.statusCode), http.expectedContentLength <= Int64(maxBytes) else { return .answered(nil) }
        var data = Data()
        do {
            for try await byte in bytes {
                data.append(byte)
                if data.count > maxBytes { return .answered(nil) }
            }
        } catch {
            return .answered(nil)
        }
        return .answered(data)
    }
}
