import AppKit
import Observation

/// The chat on screen: what is typed, the messages of the open chat, and the core's chat events turned into them.
/// The core does the work (providers, hand-offs, saving); this only draws.
@MainActor
@Observable
final class ChatController {
    private(set) var chatID: String?
    private(set) var messages: [LiveMessage] = []
    private(set) var streaming = false
    private(set) var recent: [ChatSummary] = []
    var draft = ""
    /// Files waiting to go with the next message.
    var attachments: [URL] = []

    @ObservationIgnored private let core: BuddyCore

    init(core: BuddyCore) {
        self.core = core
    }

    var hasContent: Bool { !messages.isEmpty }

    /// The first question, as the chat's title.
    var title: String {
        guard let first = messages.first(where: { $0.role == "user" })?.content else { return "Buddy" }
        let line = first.split(separator: "\n").first.map(String.init) ?? first
        return line.count > 48 ? String(line.prefix(48)) + "…" : line
    }

    func attach(_ urls: [URL]) {
        for url in urls where !attachments.contains(url) { attachments.append(url) }
    }

    func detach(_ url: URL) {
        attachments.removeAll { $0 == url }
    }

    func send() {
        var text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty || !attachments.isEmpty else { return }
        if text.isEmpty { text = attachments.count == 1 ? "Revisa este archivo." : "Revisa estos archivos." }
        let files = attachments
        draft = ""
        attachments = []
        messages.append(LiveMessage(role: "user", content: text, files: files.map(\.path)))
        messages.append(LiveMessage(role: "assistant", content: "", isStreaming: true, author: "Buddy", activity: .thinking))
        streaming = true
        do {
            chatID = try core.sendMessage(chatId: chatID, text: text, attachments: files.map(\.path))
        } catch {
            finish(failure: "No se pudo enviar: \(error)")
        }
    }

    /// Writes the last answer again.
    func regenerate() {
        guard let chatID, !streaming, let last = messages.lastIndex(where: { $0.role == "assistant" }) else { return }
        messages[last] = LiveMessage(role: "assistant", content: "", isStreaming: true, author: "Buddy", activity: .thinking)
        streaming = true
        do { try core.regenerate(chatId: chatID) } catch { finish(failure: "No se pudo rehacer: \(error)") }
    }

    func search(_ query: String) -> [ChatSummary] {
        (try? core.searchChats(query: query, limit: 60)) ?? []
    }

    func delete(_ id: String) {
        try? core.deleteChat(chatId: id)
        if id == chatID { newChat() }
        refreshRecent()
    }

    func stop() {
        guard let chatID else { return }
        core.cancelChat(chatId: chatID)
    }

    func newChat() {
        stop()
        chatID = nil
        messages = []
        streaming = false
    }

    func refreshRecent() {
        recent = (try? core.chats(limit: 12)) ?? []
    }

    func open(_ id: String) {
        stop()
        chatID = id
        streaming = false
        let agents = Dictionary(core.agents().map { ($0.id, $0.name) }, uniquingKeysWith: { a, _ in a })
        messages = ((try? core.messages(chatId: id)) ?? []).map { m in
            LiveMessage(role: m.role, content: m.text,
                        sources: m.sources.compactMap { ChatSource.make(title: $0.title, url: $0.url) },
                        author: m.role == "assistant" ? (agents[m.agent] ?? m.agent) : nil,
                        provider: m.provider,
                        files: m.attachments,
                        failed: m.failed)
        }
    }

    /// A core event for the chat on screen (others are ignored here; the history has them).
    func handle(_ event: Event) {
        switch event {
        case let .chatStarted(chatId, agent, agentName, provider) where chatId == chatID:
            update { m in
                m.author = agentName
                m.provider = provider
                if m.content.isEmpty {
                    m.activity = agent == "buddy" ? .thinking : .handoff(to: agentName)
                }
            }
        case let .chatDelta(chatId, text) where chatId == chatID:
            update { $0.content += text; $0.activity = nil }
        case let .chatTool(chatId, name, summary) where chatId == chatID:
            update { $0.activity = .tool(name, summary) }
        case let .chatSource(chatId, title, url) where chatId == chatID:
            if let source = ChatSource.make(title: title, url: url) {
                update { m in if !m.sources.contains(where: { $0.url == source.url }) { m.sources.append(source) } }
            }
        case let .chatDone(chatId, _) where chatId == chatID:
            finish(failure: nil)
        case let .chatFailed(chatId, message) where chatId == chatID:
            finish(failure: message)
        default:
            break
        }
    }

    private func finish(failure: String?) {
        update { m in
            m.isStreaming = false
            m.activity = nil
            if let failure, m.content.isEmpty { m.content = failure; m.failed = true }
        }
        streaming = false
    }

    private func update(_ change: (inout LiveMessage) -> Void) {
        guard let i = messages.lastIndex(where: { $0.role == "assistant" }) else { return }
        change(&messages[i])
    }

}

/// Core events arrive on the core's thread; this hops them to the main actor.
final class CoreEvents: EventListener, @unchecked Sendable {
    private let deliver: @MainActor (Event) -> Void

    init(_ deliver: @escaping @MainActor (Event) -> Void) {
        self.deliver = deliver
    }

    func onEvent(event: Event) {
        Task { @MainActor [deliver] in deliver(event) }
    }
}
