import Foundation

/// `mika-telegram`, the helper process that reads the user's own Telegram account (apps/windows/telegram, built on
/// grammers; the very same binary as on Windows). It lives in the app bundle at `Contents/Helpers/mika-telegram`, put
/// there by the Mika target's "Telegram helper" build phase (scripts/build-telegram-helper-macos.sh).
///
/// Twin of apps/windows/src-tauri/src/integrations/telegram.rs: started on the first need, one request at a time, each
/// with its own timeout; a helper that hangs or dies is dropped (killed) and the next request starts a fresh one. It
/// quits by itself after 15 idle minutes, or when MIKA closes its stdin (and `ProviderProcess.terminateAll` kills it
/// when MIKA quits).
///
/// What goes where: api_id and api_hash are typed by the user (Settings → Integrations → Telegram, Keychain). The session
/// (the authorisation key the login creates) arrives in `{"event":"session"}` lines and goes straight to the Keychain as
/// `telegram-session`, an item Settings never shows, sets or clears. Nothing here logs a secret, the phone number, the
/// login code or the password.
actor TelegramHelper {
    static let shared = TelegramHelper()

    static let keyAPIID = "telegram-api-id"
    static let keyAPIHash = "telegram-api-hash"
    static let keySession = "telegram-session"
    static let executableName = "mika-telegram"

    private enum Failure: Error { case said(String), timeout, gone }

    private var process: ProviderProcess?
    private var pump: Task<Void, Never>?
    private var nextID: UInt64 = 1
    private var waiting: (id: UInt64, continuation: CheckedContinuation<Result<Data, Failure>, Never>)?
    private var timer: Task<Void, Never>?
    /// One request at a time: the others wait here, in order.
    private var busy = false
    private var queue: [CheckedContinuation<Void, Never>] = []
    /// New credentials or a sign-out while a request runs: the next request starts a fresh helper.
    private var resetRequested = false

    // MARK: - Login and chats (Settings)

    func status() async -> TelegramStatus {
        guard Self.credentials() != nil else { return TelegramStatus() }
        do {
            return try Self.status(from: await call(.status, timeout: .seconds(30)))
        } catch {
            return TelegramStatus(configured: true, error: MessageError.text(error))
        }
    }

    /// Telegram sends a login code to the user's Telegram app (or by SMS).
    func sendCode(_ phone: String) async throws -> TelegramStatus {
        try Self.status(from: await call(.sendCode(phone: phone), timeout: .seconds(60)))
    }

    func signIn(_ code: String) async throws -> TelegramStatus {
        try Self.status(from: await call(.signIn(code: code), timeout: .seconds(60)))
    }

    /// The 2FA password, when the account has one. It goes to the helper and is never stored.
    func password(_ password: String) async throws -> TelegramStatus {
        try Self.status(from: await call(.password(password), timeout: .seconds(60)))
    }

    /// Closes the session on Telegram's side too, then forgets it here whatever Telegram said.
    func signOut() async {
        if Self.credentials() != nil { _ = try? await call(.signOut, timeout: .seconds(20)) }
        Self.storeSession(nil)
        reset()
    }

    /// The channels and groups the user is in, to pick from (never private chats or bots).
    func chats() async throws -> [TelegramChatOption] {
        let data = try await call(.chats(limit: 300), timeout: .seconds(60))
        guard let reply = try? JSONDecoder().decode(TelegramReply<[TelegramChatOption]>.self, from: data) else {
            throw MessageError("Telegram no respondió como se esperaba.")
        }
        return reply.result
    }

    /// The new posts of the picked chats; their photos land in `mediaDir`.
    func fetch(_ chats: [TelegramFetchChat], mediaDir: URL) async throws -> TelegramFetchResult {
        let data = try await call(.fetch(chats: chats, limit: 20, mediaDir: mediaDir.path), timeout: .seconds(180))
        return (try? JSONDecoder().decode(TelegramReply<TelegramFetchResult>.self, from: data))?.result ?? TelegramFetchResult()
    }

    /// Drops the running helper (it is killed with it), or asks the next request to do so if one is running now.
    func reset() {
        if busy { resetRequested = true } else { drop() }
    }

    // MARK: - Requests

    /// One request, starting the helper when needed. A helper that hangs or dies is dropped; the next call starts a new
    /// one. Returns the answer line.
    private func call(_ command: TelegramCommand, timeout: Duration) async throws -> Data {
        await acquire()
        defer { release() }
        if resetRequested { resetRequested = false; drop() }
        if process == nil { try await start() }
        switch await request(command, timeout: timeout) {
        case .success(let data):
            return data
        case .failure(.said(let message)):
            throw MessageError(message)
        case .failure(.timeout):
            drop()
            throw MessageError("Telegram tardó demasiado en responder.")
        case .failure(.gone):
            drop()
            throw MessageError("El lector de Telegram se cerró. Intenta de nuevo.")
        }
    }

    private func start() async throws {
        guard let credentials = Self.credentials() else {
            throw MessageError("Guarda primero tu api_id y api_hash de Telegram.")
        }
        let (apiID, apiHash) = credentials
        guard let executable = Self.executableURL() else {
            throw MessageError("No se encontró mika-telegram. Reinstala MIKA (para compilarla hace falta Rust: cargo).")
        }
        // The helper needs nothing from the user's environment.
        let helper = ProviderProcess(executable: executable, arguments: [], environment: ["PATH": "/usr/bin:/bin"],
                                     currentDirectory: FileManager.default.temporaryDirectory)
        do { try helper.start(stdin: nil, keepStdinOpen: true) } catch {
            throw MessageError("No se pudo iniciar el lector de Telegram.")
        }
        process = helper
        pump = Task {
            for await line in helper.lines { await self.received(line, from: helper) }
            await self.ended(helper)
        }
        let session = KeychainStore.shared.get(Self.keySession)
        switch await request(.connect(apiID: apiID, apiHash: apiHash, session: session), timeout: .seconds(10)) {
        case .success:
            return
        case .failure(.said(let message)):
            drop()
            throw MessageError(message)
        case .failure:
            drop()
            throw MessageError("El lector de Telegram no arrancó.")
        }
    }

    private func request(_ command: TelegramCommand, timeout: Duration) async -> Result<Data, Failure> {
        guard let helper = process else { return .failure(.gone) }
        let id = nextID
        nextID += 1
        guard let line = command.line(id: id) else { return .failure(.said("Petición no válida.")) }
        return await withCheckedContinuation { (continuation: CheckedContinuation<Result<Data, Failure>, Never>) in
            waiting = (id, continuation)
            timer = Task {
                do { try await Task.sleep(for: timeout) } catch { return }
                await self.expire(id)
            }
            helper.writeLine(line)
        }
    }

    /// One line from the helper: a session to keep, or the answer to the request that is waiting.
    private func received(_ line: String, from helper: ProviderProcess) {
        guard helper === process else { return }
        let data = Data(line.utf8)
        guard let envelope = try? JSONDecoder().decode(TelegramEnvelope.self, from: data) else { return }
        if envelope.event == "session" {
            Self.storeSession(envelope.value)
            return
        }
        guard let id = envelope.id else { return }
        if envelope.ok == true {
            resolve(id, with: .success(data))
        } else {
            resolve(id, with: .failure(.said(envelope.error ?? "Telegram no respondió.")))
        }
    }

    private func ended(_ helper: ProviderProcess) {
        guard helper === process else { return }
        process = nil
        pump = nil
        if let id = waiting?.id { resolve(id, with: .failure(.gone)) }
    }

    private func expire(_ id: UInt64) { resolve(id, with: .failure(.timeout)) }

    private func resolve(_ id: UInt64, with result: Result<Data, Failure>) {
        guard let current = waiting, current.id == id else { return }
        waiting = nil
        timer?.cancel()
        timer = nil
        current.continuation.resume(returning: result)
    }

    /// Kills the helper; whatever was waiting on it fails.
    private func drop() {
        let helper = process
        process = nil
        pump?.cancel()
        pump = nil
        if let id = waiting?.id { resolve(id, with: .failure(.gone)) }
        helper?.terminate()
    }

    private func acquire() async {
        guard busy else { busy = true; return }
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in queue.append(continuation) }
    }

    private func release() {
        if queue.isEmpty { busy = false } else { queue.removeFirst().resume() }
    }

    // MARK: - Helpers

    /// api_id and api_hash, once both are stored.
    static func credentials() -> (Int64, String)? {
        guard let rawID = KeychainStore.shared.get(keyAPIID)?.trimmingCharacters(in: .whitespacesAndNewlines),
              let id = Int64(rawID), id > 0,
              let hash = KeychainStore.shared.get(keyAPIHash)?.trimmingCharacters(in: .whitespacesAndNewlines),
              !hash.isEmpty else { return nil }
        return (id, hash)
    }

    static var hasSession: Bool { KeychainStore.shared.get(keySession) != nil }

    /// The session goes straight to the Keychain (nil: signed out). Its value is never logged.
    private static func storeSession(_ value: String?) {
        if let value, !value.isEmpty {
            KeychainStore.shared.set(keySession, value: value)
        } else {
            KeychainStore.shared.remove(keySession)
        }
    }

    private static func status(from data: Data) throws -> TelegramStatus {
        guard let reply = try? JSONDecoder().decode(TelegramReply<TelegramHelperStatus>.self, from: data) else {
            throw MessageError("Telegram no respondió como se esperaba.")
        }
        return TelegramStatus(configured: true, authorized: reply.result.authorized, step: reply.result.step ?? "idle",
                              hint: reply.result.hint, name: reply.result.name)
    }

    /// In the app bundle; in a debug build without it, the one cargo left in the Windows workspace.
    static func executableURL() -> URL? {
        var candidates = [Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/\(executableName)")]
        #if DEBUG
        let macos = URL(fileURLWithPath: #filePath)            // …/apps/macos/Sources/Integrations/Telegram/<this file>
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        for folder in ["release", "aarch64-apple-darwin/release", "x86_64-apple-darwin/release"] {
            candidates.append(macos.appendingPathComponent("../windows/target/\(folder)/\(executableName)").standardizedFileURL)
        }
        #endif
        return candidates.first { FileManager.default.isExecutableFile(atPath: $0.path) }
    }

    /// True when the helper is where MIKA looks for it (Settings says so when it is not).
    static var isInstalled: Bool { executableURL() != nil }
}
