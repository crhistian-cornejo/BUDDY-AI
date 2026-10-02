import SwiftUI

// History: every conversation with every agent, newest first, grouped by day (twin of apps/windows/src/views/history.ts).
// A click opens it in its agent's chat; the conversation that was open there goes back to the History. Each agent's memory
// can be opened from here too. The grouping and the search are pure functions, tested without a screen.

/// "hoy 14:05", "ayer", "jueves", "28 sept".
func historyWhenLabel(_ ms: UInt64, now: Date = Date(), calendar: Calendar = .current) -> String {
    guard ms > 0 else { return "" }
    let date = Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
    let days = calendar.dateComponents([.day], from: calendar.startOfDay(for: date), to: calendar.startOfDay(for: now)).day ?? 0
    let formatter = DateFormatter()
    formatter.locale = Locale(identifier: "es")
    formatter.calendar = calendar
    formatter.timeZone = calendar.timeZone
    switch days {
    case 0:
        formatter.dateFormat = "HH:mm"
        return "hoy " + formatter.string(from: date)
    case 1:
        return "ayer"
    case 2..<7:
        formatter.dateFormat = "EEEE"
        return formatter.string(from: date)
    default:
        let sameYear = calendar.component(.year, from: date) == calendar.component(.year, from: now)
        formatter.setLocalizedDateFormatFromTemplate(sameYear ? "dMMM" : "dMMMy")
        return formatter.string(from: date).replacingOccurrences(of: ".", with: "")
    }
}

// MARK: - grouping and search (pure)

enum HistoryGroup: Int, CaseIterable, Sendable {
    case today, yesterday, week, month, older

    var title: String {
        switch self {
        case .today: return "Hoy"
        case .yesterday: return "Ayer"
        case .week: return "Esta semana"
        case .month: return "Este mes"
        case .older: return "Anteriores"
        }
    }

    /// Where a conversation last touched at `ms` falls: today, yesterday, the last 2-6 days, the last 7-29, or before.
    static func of(_ ms: UInt64, now: Date = Date(), calendar: Calendar = .current) -> HistoryGroup {
        guard ms > 0 else { return .older }
        let date = Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
        let days = calendar.dateComponents([.day], from: calendar.startOfDay(for: date), to: calendar.startOfDay(for: now)).day ?? 0
        switch days {
        case ..<1: return .today
        case 1: return .yesterday
        case 2..<7: return .week
        case 7..<30: return .month
        default: return .older
        }
    }
}

/// `text` lower-cased and without accents, for matching what the user types against titles.
func historyFolded(_ text: String) -> String {
    text.folding(options: [.diacriticInsensitive, .caseInsensitive], locale: Locale(identifier: "es")).lowercased()
}

/// The entries whose title (or agent name) contains every word typed, in any order; an empty query keeps everything.
func historyMatching(_ entries: [HistoryEntry], query: String, agentName: (String) -> String) -> [HistoryEntry] {
    let words = historyFolded(query).split(whereSeparator: { $0 == " " }).map(String.init)
    guard !words.isEmpty else { return entries }
    return entries.filter { entry in
        let haystack = historyFolded(entry.title + " " + agentName(entry.agent))
        return words.allSatisfy { haystack.contains($0) }
    }
}

/// The entries in sections, newest section first and each section in the order it came (the list is already newest first).
func historyGroups(_ entries: [HistoryEntry], now: Date = Date(), calendar: Calendar = .current) -> [(group: HistoryGroup, entries: [HistoryEntry])] {
    var buckets: [HistoryGroup: [HistoryEntry]] = [:]
    for entry in entries { buckets[HistoryGroup.of(entry.updated, now: now, calendar: calendar), default: []].append(entry) }
    return HistoryGroup.allCases.compactMap { group in buckets[group].map { (group, $0) } }
}

/// The time shown on a row: the hour for today and yesterday (the section says which day), else the day.
func historyRowTime(_ ms: UInt64, group: HistoryGroup, calendar: Calendar = .current) -> String {
    guard ms > 0 else { return "" }
    switch group {
    case .today, .yesterday:
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "es")
        formatter.calendar = calendar
        formatter.timeZone = calendar.timeZone
        formatter.dateFormat = "HH:mm"
        return formatter.string(from: Date(timeIntervalSince1970: TimeInterval(ms) / 1000))
    default:
        return historyWhenLabel(ms, calendar: calendar)
    }
}

/// "1 mensaje" / "12 mensajes".
func historyCountLabel(_ count: Int) -> String { count == 1 ? "1 mensaje" : "\(count) mensajes" }


struct HistoryView: View {
    @ObservedObject var state: AppState
    @State private var filter: String?
    @State private var query = ""
    @State private var entries: [HistoryEntry] = []
    @State private var error = ""
    /// The row whose delete button was pressed once: a second press deletes it.
    @State private var confirming = ""

    private func name(_ id: String) -> String { state.hub.agents.first { $0.id == id }?.name ?? id }
    private var shown: [HistoryEntry] {
        historyMatching(entries.filter { filter == nil || $0.agent == filter }, query: query, agentName: name)
    }
    private var memoryAgent: AgentDefinition? {
        state.hub.agents.first { $0.id == (filter ?? state.hub.activeID) }
    }

    var body: some View {
        ZStack(alignment: .topLeading) {
            CardBackground(wash: nil)
            VStack(alignment: .leading, spacing: 9) {
                header
                filters
                list
            }
            .padding(.top, 12)
            .padding(.horizontal, 14)
            .padding(.bottom, 8)
        }
        .padding(.bottom, 10)
        .onAppear { if state.view == .history { load() } }
        .onChange(of: state.view) { _, view in
            if view == .history { filter = nil; query = ""; load() }
            confirming = ""
        }
    }

    // MARK: pieces

    private var header: some View {
        HStack(spacing: 10) {
            Text("Historial").font(.system(size: 14, weight: .semibold)).foregroundColor(Color(hex: "#F5F6F8"))
            Text(verbatim: shown.count == 1 ? "1 conversación" : "\(shown.count) conversaciones")
                .font(.system(size: 10.5)).foregroundColor(Color(hex: "#6B7079"))
            Spacer(minLength: 0)
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass").font(.system(size: 10.5)).foregroundColor(Color(hex: "#6B7079"))
                TextField("Buscar", text: $query)
                    .textFieldStyle(.plain)
                    .font(.system(size: 11.5))
                    .foregroundColor(Color(hex: "#F1F2F4"))
                    .frame(width: 130)
                if !query.isEmpty {
                    Button(action: { query = "" }) {
                        Image(systemName: "xmark.circle.fill").font(.system(size: 10.5)).foregroundColor(Color(hex: "#6B7079"))
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Borrar la búsqueda")
                }
            }
            .padding(.horizontal, 9)
            .frame(height: 24)
            .background(Capsule().fill(Color.white.opacity(0.07)))
            Button(action: {
                if let id = memoryAgent?.id, let failed = state.openMemory(agentID: id) { error = failed }
            }) {
                HStack(spacing: 4) {
                    Image(systemName: "brain").font(.system(size: 10))
                    Text("Memoria").font(.system(size: 10.5, weight: .medium))
                }
                .foregroundColor(Color(hex: "#B0B5BE"))
                .padding(.horizontal, 9)
                .frame(height: 24)
                .background(Capsule().fill(Color.white.opacity(0.07)))
            }
            .buttonStyle(.plain)
        }
    }

    private var filters: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 5) {
                chip(nil, "Todos")
                ForEach(state.hub.agents, id: \.id) { agent in chip(agent.id, agent.name) }
            }
            .padding(.horizontal, 3).padding(.vertical, 2)     // the selected chip's outline is not clipped by the scroll view
        }
    }

    private var list: some View {
        ScrollView(.vertical, showsIndicators: false) {
            VStack(alignment: .leading, spacing: 6) {
                if !error.isEmpty {
                    emptyState(icon: "exclamationmark.triangle", text: error, color: Color(hex: "#FF8D97"))
                } else if shown.isEmpty {
                    if entries.isEmpty {
                        emptyState(icon: "bubble.left.and.bubble.right", text: "Todavía no hay conversaciones.\nEmpieza una en el chat.", color: Color(hex: "#6B7079"))
                    } else {
                        emptyState(icon: "magnifyingglass", text: "Nada coincide con tu búsqueda.", color: Color(hex: "#6B7079"))
                    }
                }
                ForEach(historyGroups(shown), id: \.group) { section in
                    Text(section.group.title.uppercased())
                        .font(.system(size: 9.5, weight: .semibold))
                        .tracking(0.6)
                        .foregroundColor(Color(hex: "#6B7079"))
                        .padding(.horizontal, 8)
                        .padding(.top, 12).padding(.bottom, 0)
                    // `id` is "open" for every agent's open conversation: the key (agent/id) is the unique one.
                    ForEach(section.entries, id: \.key) { entry in
                        HistoryRow(entry: entry, group: section.group, agent: state.hub.agents.first { $0.id == entry.agent },
                                   confirming: confirming == entry.key,
                                   onOpen: { open(entry) }, onDelete: { remove(entry) })
                    }
                }
            }
            .padding(.bottom, 6)
        }
    }

    private func chip(_ id: String?, _ label: String) -> some View {
        let on = filter == id
        return Button(action: { filter = id; confirming = "" }) {
            Text(label).font(.system(size: 11, weight: .medium))
            .foregroundColor(on ? Color(hex: "#F5F6F8") : Color(hex: "#8E939C"))
            .padding(.horizontal, 11)
            .frame(height: 24)
            .background(Capsule().fill(on ? Color.white.opacity(0.13) : Color.white.opacity(0.04)))
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
    }

    private func emptyState(icon: String, text: String, color: Color) -> some View {
        VStack(spacing: 8) {
            Image(systemName: icon).font(.system(size: 20, weight: .light)).foregroundColor(color.opacity(0.8))
            Text(text).font(.system(size: 12)).foregroundColor(color).multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 26)
        .padding(.horizontal, 8)
    }

    // MARK: actions

    private func load() {
        error = ""
        entries = state.historyEntries()
    }

    private func open(_ entry: HistoryEntry) {
        if let failed = state.openConversation(entry) { error = failed }
    }

    private func remove(_ entry: HistoryEntry) {
        guard confirming == entry.key else { confirming = entry.key; return }
        confirming = ""
        if let failed = state.deleteConversation(entry) { error = failed } else { load() }
    }
}

/// One conversation: the agent's avatar, what it was about, who and how long, when; a trash button on hover (archived ones
/// only). The conversation that is open in a chat right now carries a green badge, and nothing else sets it apart.
private struct HistoryRow: View {
    let entry: HistoryEntry
    let group: HistoryGroup
    let agent: AgentDefinition?
    let confirming: Bool
    let onOpen: () -> Void
    let onDelete: () -> Void
    @State private var hovered = false
    @State private var trashHovered = false

    /// The agent's own mascot (its colours, hat and clothes), still: one frame, no timer.
    private var avatarTask: AgentTask {
        var task = AppState.shared.tasks.first { $0.id == AgentTask.agentTaskID(entry.agent) }
            ?? agent.map { AgentTask.pill(for: $0, provider: $0.provider) }
            ?? AgentTask(id: "history_" + entry.agent, name: entry.agent, color: "#E6E9EE", state: .idle, steps: [], source: .claudeCode)
        task.state = .idle
        return task
    }

    var body: some View {
        HStack(spacing: 12) {
            MiniBotCanvasView(task: avatarTask, animated: false)
                .frame(width: 30 / 0.6, height: 30 / 0.6)
                .frame(width: 30, height: 30)
                .id(avatarTask.lookKey)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(entry.title)
                    .font(.system(size: 12.5, weight: .medium))
                    .foregroundColor(Color(hex: "#E3E6EB"))
                    .lineLimit(1)
                    .truncationMode(.tail)
                Text(verbatim: "\(agent?.name ?? entry.agent) · \(historyCountLabel(entry.count))")
                    .font(.system(size: 10.5))
                    .foregroundColor(Color(hex: "#6B7079"))
                    .lineLimit(1)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if entry.active {
                Text("abierta")
                    .font(.system(size: 9.5, weight: .semibold))
                    .foregroundColor(Color(hex: "#34D399"))
                    .padding(.horizontal, 7).padding(.vertical, 2)
                    .background(Capsule().fill(Color(hex: "#34D399").opacity(0.14)))
            }
            Text(historyRowTime(entry.updated, group: group))
                .font(.system(size: 10.5).monospacedDigit())
                .foregroundColor(Color(hex: "#6B7079"))
                .lineLimit(1)
                .fixedSize()
            if !entry.active {
                Button(action: onDelete) {
                    Group {
                        if confirming {
                            Text("¿Borrar?").font(.system(size: 10.5, weight: .semibold)).padding(.horizontal, 7)
                        } else {
                            Image(systemName: "trash").font(.system(size: 10.5)).frame(width: 22)
                        }
                    }
                    .frame(height: 22)
                    .foregroundColor(confirming || trashHovered ? Color(hex: "#FF8D97") : Color(hex: "#6B7079"))
                    .background(RoundedRectangle(cornerRadius: 7).fill(trashHovered || confirming ? Color(hex: "#F4505E").opacity(0.15) : Color.clear))
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .opacity(hovered || confirming ? 1 : 0)
                .onHover { trashHovered = $0 }
                .accessibilityLabel("Borrar conversación")
            } else {
                Color.clear.frame(width: 22, height: 22)          // the same width, so every time lines up
            }
        }
        .padding(.horizontal, 8)
        .frame(minHeight: 48)
        .background(RoundedRectangle(cornerRadius: 10).fill(hovered ? Color.white.opacity(0.05) : Color.clear))
        .contentShape(Rectangle())
        .onTapGesture(perform: onOpen)
        .onHover { hovered = $0 }
        .animation(.easeOut(duration: 0.12), value: hovered)
    }
}
