import Foundation

/// The phone's connection to one paired machine, through the relay. The encryption is `PhoneChannel` (Rust, the
/// same code the desktop runs): this class only carries its frames over a WebSocket and pairs replies with calls.
@MainActor
@Observable
final class RemoteLink {
    enum Status: Equatable {
        case connecting
        /// The relay is reached, the machine is not connected to it (off, asleep or Buddy closed).
        case machineAway
        case ready
        case offline(String)
    }

    struct Failure: LocalizedError {
        let message: String
        var errorDescription: String? { message }
    }

    let machine: Machine
    private(set) var status: Status = .connecting
    /// One of the core's events, as its JSON object.
    var onEvent: (([String: Any]) -> Void)?
    /// The channel opened (or opened again): time to load what is on screen.
    var onReady: (() -> Void)?

    private var socket: URLSessionWebSocketTask?
    private var channel: PhoneChannel?
    private var waiting: [UInt32: CheckedContinuation<Data, Error>] = [:]
    private var nextID: UInt32 = 0
    private var attempts = 0
    private var retry: Task<Void, Never>?
    private var generation = 0

    init(machine: Machine) {
        self.machine = machine
    }

    /// `https://relay…` as the socket's address for a room, or nil when it is not a relay's address.
    nonisolated static func socketURL(relay: String, room: String) -> URL? {
        var base = relay.trimmingCharacters(in: .whitespaces)
        while base.hasSuffix("/") { base.removeLast() }
        if base.hasPrefix("https://") {
            base = "wss://" + base.dropFirst(8)
        } else if base.hasPrefix("http://127.0.0.1") || base.hasPrefix("http://localhost") {
            base = "ws://" + base.dropFirst(7)
        } else {
            return nil
        }
        return URL(string: "\(base)/rooms/\(room)/ws?role=phone")
    }

    func connect() {
        retry?.cancel()
        socket?.cancel(with: .goingAway, reason: nil)
        fail(all: "Se perdió la conexión.")
        guard let url = Self.socketURL(relay: machine.relay, room: machine.id), let key = machine.roomKey else {
            status = .offline("Faltan las claves de este equipo: emparéjalo de nuevo.")
            return
        }
        generation += 1
        let mine = generation
        status = .connecting
        var request = URLRequest(url: url)
        request.setValue("Bearer \(key)", forHTTPHeaderField: "Authorization")
        let task = URLSession.shared.webSocketTask(with: request)
        socket = task
        task.resume()
        Task { [weak self] in
            while true {
                guard let message = try? await task.receive() else { break }
                guard let self, self.generation == mine else { return }
                if case let .string(text) = message { self.received(text) }
            }
            guard let self, self.generation == mine else { return }
            self.lost()
        }
    }

    func disconnect() {
        retry?.cancel()
        generation += 1
        socket?.cancel(with: .goingAway, reason: nil)
        socket = nil
        channel = nil
        fail(all: "Desconectado.")
    }

    /// Calls the machine and decodes its answer.
    func call<T: Decodable>(_ name: String, _ args: [String: Any] = [:], as type: T.Type = T.self) async throws -> T {
        let data = try await raw(name, args)
        do {
            return try JSONDecoder().decode(T.self, from: data)
        } catch {
            throw Failure(message: "El equipo respondió algo que esta versión de la app no entiende.")
        }
    }

    /// Calls the machine for its effect only.
    func send(_ name: String, _ args: [String: Any] = [:]) async throws {
        _ = try await raw(name, args)
    }

    private func raw(_ name: String, _ args: [String: Any]) async throws -> Data {
        guard let channel, let socket, status == .ready else { throw Failure(message: "El equipo no está conectado.") }
        nextID &+= 1
        let id = nextID
        let json = String(data: (try? JSONSerialization.data(withJSONObject: args)) ?? Data("{}".utf8), encoding: .utf8) ?? "{}"
        let frames = try channel.call(id: id, name: name, argsJson: json)
        return try await withCheckedThrowingContinuation { continuation in
            waiting[id] = continuation
            Task {
                for frame in frames { try? await socket.send(.string(frame)) }
            }
            // An answer that never comes must not keep its caller waiting for ever.
            Task { [weak self] in
                try? await Task.sleep(for: .seconds(30))
                self?.waiting.removeValue(forKey: id)?.resume(throwing: Failure(message: "El equipo no respondió a tiempo."))
            }
        }
    }

    private func received(_ text: String) {
        attempts = 0
        // The relay says whether the machine is there before any channel exists: a fresh one reads it.
        let reader = channel ?? (try? fresh())
        guard let reader else { return }
        switch reader.receive(frame: text) {
        case let .peer(online):
            if online { greet() } else {
                channel = nil
                status = .machineAway
                fail(all: "El equipo se desconectó.")
            }
        case .ready:
            status = .ready
            onReady?()
        case let .frame(json):
            frame(json)
        case .broken:
            fail(all: "Se perdió el canal con el equipo.")
            greet()
        case .relay, .nothing:
            break
        }
    }

    private func fresh() throws -> PhoneChannel {
        guard let key = machine.privateKey else { throw Failure(message: "Faltan las claves de este equipo.") }
        return try PhoneChannel(privateKey: key, desktopPublic: machine.desktopPublic, room: machine.id)
    }

    /// Starts a new encrypted channel with the machine.
    private func greet() {
        guard let next = try? fresh(), let socket else { return }
        channel = next
        status = .connecting
        Task { try? await socket.send(.string(next.greeting())) }
    }

    private func frame(_ json: String) {
        guard let object = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] else { return }
        if object["k"] as? String == "event", let event = object["event"] as? [String: Any] {
            onEvent?(event)
            return
        }
        guard object["k"] as? String == "reply", let id = (object["id"] as? NSNumber)?.uint32Value, let continuation = waiting.removeValue(forKey: id) else { return }
        if let reason = object["err"] as? String {
            continuation.resume(throwing: Failure(message: reason))
        } else {
            let value = object["ok"] ?? NSNull()
            let data = (try? JSONSerialization.data(withJSONObject: value, options: .fragmentsAllowed)) ?? Data("null".utf8)
            continuation.resume(returning: data)
        }
    }

    private func fail(all message: String) {
        let pending = waiting
        waiting = [:]
        for continuation in pending.values { continuation.resume(throwing: Failure(message: message)) }
    }

    /// The socket closed: try again after 2 s, 4 s, 8 s… up to a minute (the app is in the foreground).
    private func lost() {
        channel = nil
        fail(all: "Se perdió la conexión.")
        status = .offline("Sin conexión con el relé.")
        attempts += 1
        let wait = min(pow(2.0, Double(attempts)), 60)
        retry = Task { [weak self] in
            try? await Task.sleep(for: .seconds(wait))
            guard !Task.isCancelled else { return }
            self?.connect()
        }
    }

    /// Pairs with the machine whose code was scanned: proves the scan to it, checks its proof back and keeps the
    /// keys. The machine's own name comes later, when it is asked who it is.
    static func pair(uri: String, deviceName: String) async throws -> Machine {
        let offer = try scan(uri: uri)
        guard let url = socketURL(relay: offer.relay, room: offer.room) else { throw Failure(message: "La dirección del relé no es válida.") }
        let keys = newKeys()
        var request = URLRequest(url: url)
        request.setValue("Bearer \(offer.roomKey)", forHTTPHeaderField: "Authorization")
        let socket = URLSession.shared.webSocketTask(with: request)
        socket.resume()
        defer { socket.cancel(with: .normalClosure, reason: nil) }
        try await socket.send(.string(try pairRequest(uri: uri, publicKey: keys.publicKey, name: deviceName)))
        let deadline = Date().addingTimeInterval(20)
        while Date() < deadline {
            guard case let .string(text) = try await socket.receive() else { continue }
            if let pushKey = try pairConfirm(uri: uri, keys: keys, frame: text) {
                Vault.set(keys.privateKey, "\(offer.room).private")
                Vault.set(Data(offer.roomKey.utf8), "\(offer.room).room")
                Vault.set(pushKey, "\(offer.room).push")
                return Machine(id: offer.room, name: "Equipo", platform: "", relay: offer.relay, desktopPublic: offer.desktopPublic)
            }
        }
        throw Failure(message: "El equipo no respondió. Genera un código nuevo e inténtalo otra vez.")
    }
}
