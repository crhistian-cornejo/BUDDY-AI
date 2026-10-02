import Foundation

/// The Telegram pill: every 2 minutes, while the user has it switched on, the new posts of the picked chats go into
/// the workspace of every agent that takes them (PARLEY) and the card shows them with PARLEY's latest picks. Twin of
/// `poll` and `card_data` in apps/windows/src-tauri/src/integrations/telegram.rs.
///
/// Telegram's terms forbid using Telegram data with AI models, and Settings says so before the user logs in. MIKA only
/// reads: it never sends, deletes, reacts, joins or marks anything as read.

final class TelegramPoller: Integration, @unchecked Sendable {
    static let shared = TelegramPoller()
    /// The integration name an agent puts in its agent.md to receive the posts.
    static let agentIntegration = TelegramInbox.agentIntegration

    let id = "integration_telegram"
    let displayName = "Telegram"
    let color = "#2AABEE"
    let fields = [
        IntegrationField(key: TelegramHelper.keyAPIID, placeholder: "api_id  (1234567)", isSecret: false),
        IntegrationField(key: TelegramHelper.keyAPIHash, placeholder: "api_hash  (0123abcd…)"),
    ]
    /// The session: written only by TelegramHelper, never shown in Settings.
    var internalKeys: [String] { [TelegramHelper.keySession] }
    // Picks are posted shortly before kick-off: every 2 minutes, one small request per picked chat.
    let pollInterval: TimeInterval = 120
    let firstPollDelay: TimeInterval = 8
    let dashboardURL: URL? = nil

    private let lock = NSLock()
    private var polling = false

    private init() {}

    func poll() {
        let free = lock.withLock { () -> Bool in
            guard !polling else { return false }
            polling = true
            return true
        }
        guard free else { return }      // a slow fetch is still running: this tick is skipped
        Task.detached(priority: .utility) {
            await Self.run()
            self.lock.withLock { self.polling = false }
        }
    }

    private static func run() async {
        guard TelegramHelper.credentials() != nil else { return await publishCard(error: "Configura Telegram en Ajustes.") }
        guard TelegramHelper.hasSession else { return await publishCard(error: "Inicia sesión en Telegram desde Ajustes.") }
        let (chats, dirs, stateURL) = await MainActor.run { () -> ([TelegramChat], [URL], URL) in
            let app = AppState.shared
            // The agents that take the posts (PARLEY) and the main agent, which can use them for anything else.
            let dirs = app.hub.agents.filter { $0.integration == TelegramPoller.agentIntegration || $0.id == ModelCatalog.orchestrator }
                .map { TelegramInbox.dir(workspace: app.agentStore.workspace(for: $0)) }
            return (app.telegramChats, dirs, TelegramInbox.stateURL(agentsRoot: app.agentStore.root))
        }
        guard !chats.isEmpty else { return await publishCard(error: "Elige tus canales en Ajustes.") }
        guard let first = dirs.first else { return }

        let state = TelegramInbox.loadState(stateURL)
        let request = chats.map { chat in
            TelegramFetchChat(chat: TelegramChatRef(id: chat.id, hash: Int64(chat.hash) ?? 0, title: chat.title),
                              after: state[chat.id] ?? 0)
        }
        let result: TelegramFetchResult
        do {
            result = try await TelegramHelper.shared.fetch(request, mediaDir: TelegramInbox.mediaDir(first))
        } catch {
            return await publishCard(error: MessageError.text(error))
        }
        TelegramInbox.saveState(TelegramInbox.advance(state, with: result.last), to: stateURL)
        let now = Int64(Date().timeIntervalSince1970)
        let posts = TelegramInbox.checked(result.posts)          // only `<digits>_<digits>.jpg` photo names go on
        TelegramInbox.store(posts, in: dirs, now: now)
        // New posts are stored and counted, never announced one by one: PARLEY speaks only when a review finds a real pick.
        let fresh = posts.filter { (state[$0.chatId] ?? 0) > 0 && now - $0.date < 30 * 60 }
        MikaLog.info("telegram: \(posts.count) post(s) fetched from \(chats.count) chat(s), \(fresh.count) new")
        if !fresh.isEmpty {
            let count = fresh.count
            await MainActor.run { AppState.shared.telegramUnread += count }
        }
        if Picks.bringsReviewForward(posts, now: now) {
            await MainActor.run { PicksService.shared.notePicksPosted() }
        }
        // One chat that can't be read (left, banned) doesn't hide the others: it is a warning on the card.
        let warning = result.errors.first.map { failure in
            "\(chats.first { $0.id == failure.chatId }?.title ?? ""): \(failure.error ?? "")"
        }
        await publishCard(warning: warning)
    }

    /// Reads what the card shows from disk (PARLEY's latest review, the posts of the last day) and hands it to the
    /// island with what went wrong, if anything did.
    static func publishCard(error: String? = nil, warning: String? = nil) async {
        let workspace = AgentStore(root: AgentStore.defaultRoot).workspace(id: Picks.agentID)
        var card = TelegramCard.load(inbox: TelegramInbox.dir(workspace: workspace), picks: PicksLedger.dir(workspace: workspace),
                                     now: Int64(Date().timeIntervalSince1970))
        card.error = error
        card.warning = warning
        let ready = card
        await MainActor.run { AppState.shared.telegramCard = ready }
    }
}
