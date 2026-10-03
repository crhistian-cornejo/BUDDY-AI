import AppKit
import Observation

/// What the island shows, decided from core events: one notice at a time (the rest wait), sessions of Claude Code and
/// Codex, and whether the pointer is over it.
@MainActor
@Observable
final class NotchModel {
    enum Mode: Equatable { case idle, notice, open, drop }
    enum Tab: String, CaseIterable {
        case home = "Inicio", files = "Archivos", utilities = "Utilidades"
        var symbol: String {
            switch self { case .home: return "house.fill"; case .files: return "plus"; case .utilities: return "square.grid.2x2" }
        }
    }
    var tab: Tab = .home
    var tools: NotchTools?
    var battery: NotchBattery?
    var appointment: Appointment?
    var calendarError = ""
    var toolMessage = ""
    var status: NotchStatus?
    var system: NotchSystemMonitor?
    @ObservationIgnored private var statusTask: Task<Void, Never>?

    /// Where a session runs: its folder and the app (bundle id) holding its terminal, to bring the user back to it.
    struct Place: Equatable {
        var cwd: String
        var terminal: String
    }

    struct Notice: Identifiable, Equatable {
        enum Kind: Equatable { case approval(requestID: String, canAllow: Bool), finished, waiting, failed }
        let id = UUID()
        var kind: Kind
        var agent: String
        var title: String
        var detail: String
        var command: String = ""
        /// The rule «Permitir siempre» would save («git status»); empty: not offered.
        var always: String = ""
        /// A click on the card goes here (a session's terminal) instead of only closing it.
        var place: Place?
    }

    struct Session: Identifiable, Equatable {
        var id: String
        var agent: String
        var project: String
        var state: String
        var updated = Date()
    }

    private(set) var notice: Notice?
    private(set) var sessions: [Session] = []
    private(set) var hovering = false
    var pinned = false
    private(set) var collapsedByUser = false
    /// What Spotify or Music is playing (read only while the island is open).
    var nowPlaying: NowPlaying?
    /// When the reading above was taken (the progress bar runs from it).
    var nowPlayingAt = Date()
    var focus: FocusStatus?
    var shortcuts: [Shortcut] = []
    /// Files just dropped on the island, waiting for what to do with them.
    var dropped: [URL] = []
    /// Files are being dragged over the island.
    var dragging = false
    /// What is used of each plan.
    var usage: [ProviderUsage] = []
    /// Buddy is answering in the chat (the ears show it while the chat is closed).
    var buddyBusy = false
    /// What Buddy's turn is doing (`activity::of_tool` in the core): the ear shows its icon, the tooltip its label.
    var buddyActivity: (kind: String, label: String)?
    /// What a player says is playing, from its own change notifications (no polling): for the ears.
    var earTrack: EarTrack?

    struct EarTrack: Equatable {
        var title: String
        var artist: String
        var app: String
        var playing: Bool
    }
    @ObservationIgnored private var queue: [Notice] = []
    @ObservationIgnored private var dismissTask: Task<Void, Never>?

    /// Seconds an ordinary notice stays (an approval stays until it is answered).
    static let noticeSeconds: Double = 6

    var mode: Mode {
        if let notice, notice.isApproval || pinned || (hovering && !collapsedByUser) { return .notice }
        if dragging { return .drop }
        return pinned || (hovering && !collapsedByUser) ? .open : .idle
    }

    func acceptTools(_ tools: NotchTools) {
        self.tools = tools
        dropped = tools.files.map { URL(fileURLWithPath: $0.path) }
    }

    func showStatus(_ status: NotchStatus, seconds: Double = 2.4) {
        self.status = status
        statusTask?.cancel()
        statusTask = Task { [weak self] in
            do { try await Task.sleep(for: .seconds(seconds)) } catch { return }
            guard let self, !Task.isCancelled, self.status?.id == status.id else { return }
            self.status = nil
        }
    }

    func revealFiles() {
        tab = .files
        dragging = false
        collapsedByUser = false
        hovering = true
    }

    func setHovering(_ value: Bool) {
        hovering = value
        if !value { collapsedByUser = false }
        // A notice under the pointer stays; it leaves on its own once the pointer goes.
        if !value, let notice, !notice.isApproval { scheduleDismiss(after: 2) }
    }

    /// Closing tools never answers or hides a permission request, or discards files.
    func collapse() {
        pinned = false
        collapsedByUser = true
        if notice?.isApproval == false { dismiss() }
    }

    var activityLabel: String {
        if let session = activeSession, session.state == "waiting" {
            return "\(AgentNames.name(session.agent)) espera tu respuesta"
        }
        if buddyBusy { return buddyActivity?.label ?? "Buddy está respondiendo" }
        let working = sessions.filter { $0.state == "working" }.count
        if working > 0 { return working == 1 ? "1 agente trabajando" : "\(working) agentes trabajando" }
        if focus?.running == true { return "Enfoque en curso" }
        if let earTrack, earTrack.playing { return "Sonando en \(earTrack.app)" }
        return "Todo a mano"
    }

    func show(_ notice: Notice) {
        if self.notice == nil {
            present(notice)
        } else if notice.isApproval, self.notice?.isApproval == false {
            // A permission takes the place of a plain notice at once: it is what blocks the session.
            dismissTask?.cancel()
            present(notice)
        } else if notice.isApproval {
            // A permission jumps ahead of plain notices: it blocks someone's session.
            queue.insert(notice, at: queue.firstIndex { !$0.isApproval } ?? queue.endIndex)
        } else {
            queue.append(notice)
        }
    }

    func dismiss(_ id: UUID? = nil) {
        guard let notice, id == nil || notice.id == id else {
            queue.removeAll { $0.id == id }
            return
        }
        _ = notice
        dismissTask?.cancel()
        self.notice = nil
        if !queue.isEmpty { present(queue.removeFirst()) }
    }

    /// The approval with this request id left (answered here or elsewhere, timed out).
    func closeApproval(_ requestID: String) {
        if case .approval(requestID, _) = notice?.kind { dismiss() }
        queue.removeAll { if case .approval(requestID, _) = $0.kind { return true } else { return false } }
    }

    func update(session id: String, agent: String, project: String, state: String) {
        if state == "ended" {
            sessions.removeAll { $0.id == id }
            return
        }
        if let i = sessions.firstIndex(where: { $0.id == id }) {
            sessions[i].state = state
            sessions[i].updated = Date()
        } else {
            sessions.insert(Session(id: id, agent: agent, project: project, state: state), at: 0)
        }
    }

    private func present(_ notice: Notice) {
        self.notice = notice
        if !notice.isApproval { scheduleDismiss(after: Self.noticeSeconds) }
    }

    private func scheduleDismiss(after seconds: Double) {
        dismissTask?.cancel()
        let id = notice?.id
        dismissTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(seconds))
            guard let self, !Task.isCancelled, !self.hovering else { return }
            self.dismiss(id)
        }
    }
}

extension NotchModel.Notice {
    var isApproval: Bool { if case .approval = kind { return true } else { return false } }

    var agentName: String { agent == "buddy" ? "Buddy" : agent == "niko" ? "Niko" : AgentNames.name(agent) }
}

/// How the notch and the settings name a coding agent of the user's (the core's `agent` / `provider` ids) and which
/// provider mark they show for it.
enum AgentNames {
    /// A session's agent: "Claude Code", "Codex", "Gemini".
    static func name(_ agent: String) -> String {
        switch agent {
        case "codex": return "Codex"
        case "antigravity": return "Gemini"
        default: return "Claude Code"
        }
    }

    /// A plan's provider: "Claude", "Codex", "Gemini".
    static func plan(_ provider: String) -> String {
        switch provider {
        case "codex": return "Codex"
        case "antigravity": return "Gemini"
        default: return "Claude"
        }
    }

    /// The `ProviderMark` id for an agent.
    static func mark(_ agent: String) -> String {
        agent == "codex" || agent == "antigravity" ? agent : "claude"
    }
}
