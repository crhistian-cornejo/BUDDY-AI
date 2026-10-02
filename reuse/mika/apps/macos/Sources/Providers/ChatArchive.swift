import Foundation

// Past conversations and each agent's memory, as plain files in the agent's folder: `chat.json` (the open conversation)
// and `chat.meta.json` (when it started, how much of it is in memory), `chats/<id>.json` (archived conversations, listed
// in the History view) and `memory.md` (short notes the agent gets with every turn, so it remembers what it did before;
// the user can edit it). Twin of apps/windows/src-tauri/src/services/chat_store.rs.

/// An archived conversation.
struct Conversation: Codable, Equatable, Sendable {
    var id: String
    var agent: String
    var title: String
    var created: UInt64
    var updated: UInt64
    /// How many of the messages are already summarised in the agent's memory.
    var memorized: Int
    var messages: [ChatMessage]

    init(id: String, agent: String, title: String, created: UInt64, updated: UInt64, memorized: Int, messages: [ChatMessage]) {
        self.id = id; self.agent = agent; self.title = title; self.created = created; self.updated = updated
        self.memorized = memorized; self.messages = messages
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        agent = try c.decode(String.self, forKey: .agent)
        title = try c.decode(String.self, forKey: .title)
        created = try c.decode(UInt64.self, forKey: .created)
        updated = try c.decode(UInt64.self, forKey: .updated)
        memorized = try c.decodeIfPresent(Int.self, forKey: .memorized) ?? 0
        messages = try c.decode([ChatMessage].self, forKey: .messages)
    }
}

/// One row of the History view.
struct HistoryEntry: Identifiable, Equatable, Sendable {
    /// The archived id, or "open" for the conversation that is open in that agent's chat right now.
    var id: String
    var agent: String
    var title: String
    var updated: UInt64
    var count: Int
    var active: Bool
    var key: String { agent + "/" + id }
}

/// The open conversation: when it started and how many of its messages are already in memory.
struct ChatMeta: Codable, Equatable, Sendable {
    var created: UInt64 = 0
    var memorized: Int = 0

    init(created: UInt64 = 0, memorized: Int = 0) { self.created = created; self.memorized = memorized }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        created = try c.decodeIfPresent(UInt64.self, forKey: .created) ?? 0
        memorized = try c.decodeIfPresent(Int.self, forKey: .memorized) ?? 0
    }
}

enum ChatArchive {
    /// What `memory.md` may hold; the oldest notes go first.
    static let memoryMax = 6000
    static let maxArchived = 200

    static func nowMs(_ date: Date = Date()) -> UInt64 { UInt64(max(0, date.timeIntervalSince1970 * 1000)) }

    private static func folder(_ store: AgentStore, _ agentID: String) -> URL {
        store.root.appendingPathComponent(agentID, isDirectory: true)
    }
    static func chatsDir(_ store: AgentStore, _ agentID: String) -> URL { folder(store, agentID).appendingPathComponent("chats", isDirectory: true) }
    static func metaURL(_ store: AgentStore, _ agentID: String) -> URL { folder(store, agentID).appendingPathComponent("chat.meta.json") }
    static func memoryURL(_ store: AgentStore, _ agentID: String) -> URL { folder(store, agentID).appendingPathComponent("memory.md") }

    /// Archived ids are the start time in milliseconds: digits only, so they can never point outside the folder.
    static func validID(_ id: String) -> Bool {
        !id.isEmpty && id.count <= 20 && id.allSatisfy { $0.isASCII && $0.isNumber }
    }

    static func validAgent(_ id: String) -> Bool { id.range(of: #"^[a-z0-9][a-z0-9_-]*$"#, options: .regularExpression) != nil }

    static func meta(_ store: AgentStore, _ agentID: String) -> ChatMeta {
        (try? Data(contentsOf: metaURL(store, agentID))).flatMap { try? JSONDecoder().decode(ChatMeta.self, from: $0) } ?? ChatMeta()
    }

    static func setMeta(_ store: AgentStore, _ agentID: String, _ meta: ChatMeta) {
        guard let data = try? JSONEncoder().encode(meta) else { return }
        try? FileManager.default.createDirectory(at: folder(store, agentID), withIntermediateDirectories: true,
                                                 attributes: [.posixPermissions: 0o700])
        try? data.write(to: metaURL(store, agentID), options: .atomic)
    }

    /// The conversation's name in the History view: the first thing the user asked.
    static func title(of messages: [ChatMessage]) -> String {
        let first = messages.first { $0.role == .user }?.content ?? "Conversación"
        let line = first.components(separatedBy: "\n").map { $0.trimmingCharacters(in: .whitespaces) }
            .first { !$0.isEmpty } ?? "Conversación"
        return line.count > 80 ? String(line.prefix(80)) + "…" : line
    }

    /// Moves the open conversation into `chats/`. Returns it (with its id) when there was one.
    static func archive(_ store: AgentStore, _ agentID: String, messages: [ChatMessage], now: UInt64 = nowMs()) -> Conversation? {
        guard !messages.isEmpty, validAgent(agentID) else { return nil }
        let open = Self.meta(store, agentID)
        let created = open.created > 0 ? open.created : now
        let dir = chatsDir(store, agentID)
        guard (try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true,
                                                         attributes: [.posixPermissions: 0o700])) != nil else { return nil }
        // Two archives in the same millisecond must not overwrite each other.
        var id = created
        while FileManager.default.fileExists(atPath: dir.appendingPathComponent("\(id).json").path) { id += 1 }
        let kept = messages.filter { !$0.isGeneratingImage }.map { var m = $0; m.isStreaming = false; m.status = nil; return m }
        let conversation = Conversation(id: String(id), agent: agentID, title: title(of: kept), created: created, updated: now,
                                        memorized: min(open.memorized, kept.count), messages: kept)
        guard let data = try? JSONEncoder().encode(conversation),
              (try? data.write(to: dir.appendingPathComponent("\(id).json"), options: .atomic)) != nil else { return nil }
        setMeta(store, agentID, ChatMeta(created: now, memorized: 0))
        prune(store, agentID)
        return conversation
    }

    private static func prune(_ store: AgentStore, _ agentID: String) {
        guard let files = try? FileManager.default.contentsOfDirectory(at: chatsDir(store, agentID), includingPropertiesForKeys: nil)
        else { return }
        let archived = files.filter { $0.pathExtension == "json" }.sorted { $0.lastPathComponent < $1.lastPathComponent }
        guard archived.count > maxArchived else { return }
        archived.prefix(archived.count - maxArchived).forEach { try? FileManager.default.removeItem(at: $0) }
    }

    static func read(_ store: AgentStore, _ agentID: String, id: String) -> Conversation? {
        guard validAgent(agentID), validID(id),
              let data = try? Data(contentsOf: chatsDir(store, agentID).appendingPathComponent("\(id).json")),
              data.count <= 4_000_000 else { return nil }
        return try? JSONDecoder().decode(Conversation.self, from: data)
    }

    /// Takes an archived conversation out of the archive (it becomes the open one).
    static func take(_ store: AgentStore, _ agentID: String, id: String) -> Conversation? {
        guard let conversation = read(store, agentID, id: id) else { return nil }
        try? FileManager.default.removeItem(at: chatsDir(store, agentID).appendingPathComponent("\(id).json"))
        setMeta(store, agentID, ChatMeta(created: conversation.created, memorized: conversation.memorized))
        return conversation
    }

    @discardableResult
    static func delete(_ store: AgentStore, _ agentID: String, id: String) -> Bool {
        guard validAgent(agentID), validID(id) else { return false }
        return (try? FileManager.default.removeItem(at: chatsDir(store, agentID).appendingPathComponent("\(id).json"))) != nil
    }

    /// Every conversation of every agent, newest first: the open ones and the archived ones.
    static func list(_ store: AgentStore, agents: [AgentDefinition], open: (String) -> [ChatMessage]) -> [HistoryEntry] {
        var out: [HistoryEntry] = []
        for agent in agents {
            let current = open(agent.id)
            if !current.isEmpty {
                let modified = (try? FileManager.default.attributesOfItem(atPath: store.chatFile(for: agent).path))?[.modificationDate] as? Date
                out.append(HistoryEntry(id: "open", agent: agent.id, title: title(of: current), updated: modified.map { nowMs($0) } ?? 0,
                                        count: current.count, active: true))
            }
            guard let files = try? FileManager.default.contentsOfDirectory(at: chatsDir(store, agent.id), includingPropertiesForKeys: nil)
            else { continue }
            for file in files where file.pathExtension == "json" {
                let id = file.deletingPathExtension().lastPathComponent
                guard validID(id), let c = read(store, agent.id, id: id) else { continue }
                out.append(HistoryEntry(id: c.id, agent: agent.id, title: c.title, updated: c.updated, count: c.messages.count, active: false))
            }
        }
        return out.sorted { $0.updated > $1.updated }
    }

    // MARK: - memory

    static func readMemory(_ store: AgentStore, _ agentID: String) -> String {
        ((try? String(contentsOf: memoryURL(store, agentID), encoding: .utf8)) ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Adds notes at the end of `memory.md`, dropping the oldest lines while it is over the limit.
    static func appendMemory(_ store: AgentStore, _ agentID: String, notes: String) {
        let notes = notes.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !notes.isEmpty, validAgent(agentID) else { return }
        var text = readMemory(store, agentID)
        // Each note is added on its own: one that says what another already says only renews its date.
        for note in AgentMemory.notes(in: notes) {
            text = AgentMemory.adding(date: note.date, text: note.text, pinned: note.pinned, to: text, max: memoryMax)
        }
        writeMemory(store, agentID, text)
    }

    static func writeMemory(_ store: AgentStore, _ agentID: String, _ text: String) {
        guard validAgent(agentID) else { return }
        try? FileManager.default.createDirectory(at: folder(store, agentID), withIntermediateDirectories: true,
                                                 attributes: [.posixPermissions: 0o700])
        try? (text.trimmingCharacters(in: .whitespacesAndNewlines) + "\n").write(to: memoryURL(store, agentID), atomically: true, encoding: .utf8)
    }

    /// Takes the notes on those lines out of the memory. Returns what was removed.
    @discardableResult
    static func removeMemory(_ store: AgentStore, _ agentID: String, ids: Set<Int>) -> [MemoryNote] {
        let file = readMemory(store, agentID)
        let removed = AgentMemory.notes(in: file).filter { ids.contains($0.id) }
        guard !removed.isEmpty else { return [] }
        writeMemory(store, agentID, AgentMemory.removing(ids: Set(removed.map(\.id)), from: file))
        return removed
    }

    static func trimMemory(_ text: String, max: Int) -> String {
        var lines = text.components(separatedBy: "\n")
        while lines.count > 1, lines.reduce(0, { $0 + $1.utf8.count + 1 }) > max { lines.removeFirst() }
        return lines.joined(separator: "\n")
    }

    /// The block added to a system prompt that is built for one run (an automatic turn): the notes that matter for `query`.
    /// Chat turns carry theirs in the message instead (AgentChat.turnMemory), so a changing memory never changes the instructions.
    static func memoryPrompt(_ store: AgentStore, _ agentID: String, query: String? = nil) -> String {
        let block = AgentMemory.block(file: readMemory(store, agentID), query: query).trimmingCharacters(in: .whitespacesAndNewlines)
        return block.isEmpty ? "" : "\n\n" + block
    }

    /// What the agent is asked when a conversation is archived: a few short notes worth remembering.
    static func summaryPrompt(_ messages: [ChatMessage]) -> String {
        var out = "Resume en 1 a 3 viñetas muy cortas (máximo 160 caracteres cada una) lo que conviene recordar de esta "
            + "conversación para la próxima vez: qué pidió el usuario, qué hiciste o entregaste, decisiones y datos útiles sobre el usuario. "
            + "Escribe solo las viñetas, empezando cada una con «- ». Si no hay nada que valga la pena recordar, responde «-».\n\nConversación (datos):\n"
        for m in messages.suffix(24) {
            let who = m.role == .user ? "Usuario" : (m.agent.map { $0.uppercased() } ?? "Tú")
            out += "\(who): \(String(m.content.prefix(1200)))\n"
        }
        return out
    }

    /// Keeps only well-formed bullets from the model's answer, dated.
    static func summaryNotes(_ answer: String, date: String) -> String {
        answer.components(separatedBy: "\n")
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .compactMap { line -> String? in
                for bullet in ["- ", "* ", "• "] where line.hasPrefix(bullet) { return String(line.dropFirst(bullet.count)) }
                return nil
            }
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
            .prefix(3)
            .map { "- \(date): \(String($0.prefix(200)))" }
            .joined(separator: "\n")
    }

    /// The day as YYYY-MM-DD in the user's time zone.
    static func dateLabel(_ date: Date = Date(), timeZone: TimeZone = .current) -> String {
        let formatter = DateFormatter()
        formatter.calendar = Calendar(identifier: .gregorian)
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = timeZone
        formatter.dateFormat = "yyyy-MM-dd"
        return formatter.string(from: date)
    }

    /// The note the agent MIKA passed a request to keeps about it, even though the conversation is MIKA's.
    static func handoffNote(task: String, date: String) -> String {
        "- \(date): MIKA te pasó: \(String((task.components(separatedBy: "\n").first ?? "").prefix(150)))"
    }

    /// The answer of a `codex exec --json` run: the text of its agent messages.
    static func codexExecText(_ lines: [String]) -> String {
        lines.compactMap { line -> String? in
            guard let obj = (try? JSONSerialization.jsonObject(with: Data(line.utf8))) as? [String: Any],
                  obj["type"] as? String == "item.completed", let item = obj["item"] as? [String: Any],
                  item["type"] as? String == "agent_message" else { return nil }
            return item["text"] as? String
        }.joined(separator: "\n\n")
    }
}
