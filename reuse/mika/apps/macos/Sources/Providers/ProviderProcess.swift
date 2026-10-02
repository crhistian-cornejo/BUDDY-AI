import Foundation

enum ProviderEnvironment {
    static let blocked: Set<String> = [
        "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL", "CLAUDE_CODE_OAUTH_TOKEN",
        "OPENAI_API_KEY", "CODEX_API_KEY", "OPENAI_BASE_URL",
    ]

    /// A configured API key must never silently change subscription billing.
    static func scrubbed(_ env: [String: String]) -> [String: String] { env.filter { !blocked.contains($0.key) } }
}

enum CLILocator {
    /// A Finder-launched app has a minimal PATH, so the usual install folders are searched as well.
    static func find(_ name: String,
                     environment: [String: String] = ProcessInfo.processInfo.environment,
                     home: URL = FileManager.default.homeDirectoryForCurrentUser) -> URL? {
        var dirs = (environment["PATH"] ?? "").split(separator: ":").map(String.init)
        dirs += [home.appendingPathComponent(".local/bin").path, home.appendingPathComponent(".claude/local").path,
                 "/opt/homebrew/bin", "/usr/local/bin"]
        for dir in dirs where !dir.isEmpty {
            let url = URL(fileURLWithPath: dir).appendingPathComponent(name)
            if FileManager.default.isExecutableFile(atPath: url.path) { return url }
        }
        return nil
    }
}

/// One child process with line-by-line stdout, bounded stderr, SIGINT, and a registry so that quitting MIKA kills it.
///
/// Reading is done by dedicated threads, not by awaiting end-of-file: if the child leaves a grandchild holding the
/// pipe open, `lines` still finishes shortly after the child itself exits.
final class ProviderProcess: @unchecked Sendable {
    /// Time given to the readers to drain buffered output once the child has exited.
    private static let drainGrace: TimeInterval = 0.4

    private let process = Process()
    private let outPipe = Pipe()
    private let errPipe = Pipe()
    private let inPipe = Pipe()
    private let lock = NSLock()
    private var stderrBuffer = Data()
    private var exitCode: Int32?
    private var waiters: [CheckedContinuation<Int32, Never>] = []
    private let errDrained = DispatchGroup()
    private let lineContinuation: AsyncStream<String>.Continuation

    /// Complete stdout lines. Single consumer.
    let lines: AsyncStream<String>

    private static let registryLock = NSLock()
    nonisolated(unsafe) private static var live: [ObjectIdentifier: ProviderProcess] = [:]
    /// Writing to a child that already exited raises SIGPIPE, which would kill the app.
    private static let ignoreSigpipe: Void = { _ = signal(SIGPIPE, SIG_IGN) }()

    init(executable: URL, arguments: [String], environment: [String: String], currentDirectory: URL) {
        process.executableURL = executable
        process.arguments = arguments
        process.environment = environment
        process.currentDirectoryURL = currentDirectory
        process.standardOutput = outPipe
        process.standardError = errPipe
        process.standardInput = inPipe
        (lines, lineContinuation) = AsyncStream<String>.makeStream()
    }

    var stderrText: String {
        lock.withLock { String(decoding: stderrBuffer, as: UTF8.self) }
    }

    func start(stdin: Data?, keepStdinOpen: Bool = false) throws {
        _ = Self.ignoreSigpipe
        let continuation = lineContinuation
        process.terminationHandler = { [weak self] proc in
            guard let self else { return }
            let status = proc.terminationStatus
            self.lock.withLock { self.exitCode = status }
            // Waiting for the readers happens on a low-priority queue: this handler runs at a high QoS
            // and must not block on lower-priority threads.
            DispatchQueue.global(qos: .utility).async {
                // Let the readers drain what is already buffered; never wait on a pipe a grandchild may hold open.
                _ = self.errDrained.wait(timeout: .now() + Self.drainGrace)
                DispatchQueue.global().asyncAfter(deadline: .now() + Self.drainGrace) { continuation.finish() }
                // Leave the registry before waking anyone, so a caller that wakes up sees it already clean.
                Self.registryLock.withLock { Self.live[ObjectIdentifier(self)] = nil }
                let waiting: [CheckedContinuation<Int32, Never>] = self.lock.withLock {
                    let w = self.waiters
                    self.waiters = []
                    return w
                }
                waiting.forEach { $0.resume(returning: status) }
            }
        }
        try process.run()
        Self.registryLock.withLock { Self.live[ObjectIdentifier(self)] = self }

        let outHandle = outPipe.fileHandleForReading
        Thread.detachNewThread {
            var pending = Data()
            while let chunk = Self.readAvailable(outHandle), !chunk.isEmpty {
                pending.append(chunk)
                while let newline = pending.firstIndex(of: 0x0A) {
                    var line = pending[pending.startIndex..<newline]
                    if line.last == 0x0D { line = line.dropLast() }
                    continuation.yield(String(decoding: line, as: UTF8.self))
                    pending = Data(pending[(newline + 1)...])
                }
            }
            if !pending.isEmpty { continuation.yield(String(decoding: pending, as: UTF8.self)) }
            continuation.finish()
        }

        let errHandle = errPipe.fileHandleForReading
        errDrained.enter()
        Thread.detachNewThread { [weak self] in
            defer { self?.errDrained.leave() }
            while let chunk = Self.readAvailable(errHandle), !chunk.isEmpty {
                guard let self else { return }
                self.lock.withLock { if self.stderrBuffer.count < 32_768 { self.stderrBuffer.append(chunk) } }
            }
        }

        let inHandle = inPipe.fileHandleForWriting
        if keepStdinOpen { return }
        Thread.detachNewThread {
            if let stdin { try? inHandle.write(contentsOf: stdin) }
            try? inHandle.close()
        }
    }

    /// Returns whatever is available as soon as there is something (FileHandle.read(upToCount:) would wait for the
    /// whole count on a pipe). Empty data means end of file; nil means a read error.
    private static func readAvailable(_ handle: FileHandle) -> Data? {
        var buffer = [UInt8](repeating: 0, count: 4096)
        while true {
            let n = read(handle.fileDescriptor, &buffer, buffer.count)
            if n > 0 { return Data(buffer[0..<n]) }
            if n == 0 { return Data() }
            if errno == EINTR { continue }
            return nil
        }
    }

    /// Writes one line to a child started with `keepStdinOpen: true`.
    func writeLine(_ line: String) {
        try? inPipe.fileHandleForWriting.write(contentsOf: Data((line + "\n").utf8))
    }

    func interrupt() { if process.isRunning { kill(process.processIdentifier, SIGINT) } }
    func terminate() { if process.isRunning { process.terminate() } }

    /// Kills the process and everything it started (a shell command may have launched others).
    func killTree() {
        guard process.isRunning else { return }
        var all: [Int32] = []
        var frontier = [process.processIdentifier]
        while let pid = frontier.popLast() {
            all.append(pid)
            frontier += Self.children(of: pid)
        }
        for pid in all.reversed() { kill(pid, SIGKILL) }
    }

    private static func children(of pid: Int32) -> [Int32] {
        let pgrep = Process()
        pgrep.executableURL = URL(fileURLWithPath: "/usr/bin/pgrep")
        pgrep.arguments = ["-P", String(pid)]
        let pipe = Pipe()
        pgrep.standardOutput = pipe
        pgrep.standardError = FileHandle.nullDevice
        guard (try? pgrep.run()) != nil else { return [] }
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        pgrep.waitUntilExit()
        return String(decoding: data, as: UTF8.self).split(whereSeparator: \.isNewline).compactMap { Int32($0) }
    }

    func waitUntilExit() async -> Int32 {
        await withCheckedContinuation { (cont: CheckedContinuation<Int32, Never>) in
            let code: Int32? = lock.withLock {
                if let exitCode { return exitCode }
                waiters.append(cont)
                return nil
            }
            if let code { cont.resume(returning: code) }
        }
    }

    /// Children currently running (used to prove that quitting MIKA leaves none behind).
    static var liveCount: Int { registryLock.withLock { live.count } }

    static func terminateAll() {
        let all = registryLock.withLock { Array(live.values) }
        all.forEach { $0.terminate() }
    }
}
