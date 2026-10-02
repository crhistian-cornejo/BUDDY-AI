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

    @ObservationIgnored private let core: BuddyCore

    init(core: BuddyCore) {
        self.core = core
    }

    var hasContent: Bool { !messages.isEmpty }

    func send() {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        draft = ""
        messages.append(LiveMessage(role: "user", content: text))
        messages.append(LiveMessage(role: "assistant", content: "", isStreaming: true, status: "Pensando…"))
        streaming = true
        do {
            chatID = try core.sendMessage(chatId: chatID, text: text)
        } catch {
            finish(failure: "No se pudo enviar: \(error)")
        }
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
                        author: m.role == "assistant" ? Self.author(agents[m.agent] ?? m.agent, m.provider) : nil,
                        failed: m.failed)
        }
    }

    /// A core event for the chat on screen (others are ignored here; the history has them).
    func handle(_ event: Event) {
        switch event {
        case let .chatStarted(chatId, _, agentName, provider) where chatId == chatID:
            update { $0.author = Self.author(agentName, provider); $0.status = $0.content.isEmpty ? "Pensando…" : $0.status }
        case let .chatDelta(chatId, text) where chatId == chatID:
            update { $0.content += text; $0.status = nil }
        case let .chatTool(chatId, name, summary) where chatId == chatID:
            update { $0.status = Self.status(name, summary) }
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
            m.status = nil
            if let failure, m.content.isEmpty { m.content = failure; m.failed = true }
        }
        streaming = false
    }

    private func update(_ change: (inout LiveMessage) -> Void) {
        guard let i = messages.lastIndex(where: { $0.role == "assistant" }) else { return }
        change(&messages[i])
    }

    static func author(_ name: String, _ provider: String?) -> String {
        guard let provider, !provider.isEmpty else { return name }
        let p = ["claude": "Claude", "codex": "Codex", "antigravity": "Gemini"][provider] ?? provider
        return "\(name) · \(p)"
    }

    static func status(_ tool: String, _ summary: String) -> String {
        switch tool {
        case "WebSearch": return summary.isEmpty ? "Buscando en la web…" : "Buscando: \(summary)"
        case "WebFetch": return "Leyendo \(WebHost.of(summary) ?? "una página")…"
        case "Cambio": return summary
        default: return "Trabajando…"
        }
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
