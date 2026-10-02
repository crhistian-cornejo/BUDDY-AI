import AppKit
import SwiftUI

// The history search, in the centre of the screen like Spotlight: type to filter (the core matches every word in titles
// and messages, ignoring accents), arrows to move, return to open, esc to close. Grouping by day ported from MIKA's
// HistoryView (MIT, revision d050bc5).

enum HistoryDay: Int, CaseIterable, Sendable {
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

    static func of(_ seconds: Int64, now: Date = Date(), calendar: Calendar = .current) -> HistoryDay {
        let date = Date(timeIntervalSince1970: TimeInterval(seconds))
        let days = calendar.dateComponents([.day], from: calendar.startOfDay(for: date), to: calendar.startOfDay(for: now)).day ?? 0
        switch days {
        case ..<1: return .today
        case 1: return .yesterday
        case 2..<7: return .week
        case 7..<30: return .month
        default: return .older
        }
    }

    /// "14:05", "ayer", "jueves", "28 sept".
    static func label(_ seconds: Int64, now: Date = Date(), calendar: Calendar = .current) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(seconds))
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "es")
        formatter.calendar = calendar
        switch of(seconds, now: now, calendar: calendar) {
        case .today: formatter.dateFormat = "HH:mm"
        case .yesterday: return "ayer"
        case .week: formatter.dateFormat = "EEEE"
        case .month, .older: formatter.setLocalizedDateFormatFromTemplate("dMMM")
        }
        return formatter.string(from: date).replacingOccurrences(of: ".", with: "")
    }

    /// The chats in day sections, in the order they came (newest first).
    static func groups(_ chats: [ChatSummary], now: Date = Date()) -> [(day: HistoryDay, chats: [ChatSummary])] {
        var buckets: [HistoryDay: [ChatSummary]] = [:]
        for chat in chats { buckets[of(chat.updatedAt, now: now), default: []].append(chat) }
        return allCases.compactMap { day in buckets[day].map { (day, $0) } }
    }
}

struct HistorySearchView: View {
    let chat: ChatController
    var onOpen: (String) -> Void
    var onClose: () -> Void

    @State private var query = ""
    @State private var results: [ChatSummary] = []
    @State private var selected: String?
    @FocusState private var focused: Bool

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 16, weight: .medium))
                    .foregroundStyle(.secondary)
                TextField("Buscar en tus chats", text: $query)
                    .textFieldStyle(.plain)
                    .font(.title3)
                    .focused($focused)
                    .onSubmit(openSelected)
                if !query.isEmpty {
                    Button { query = "" } label: { Image(systemName: "xmark.circle.fill") }
                        .buttonStyle(.borderless)
                        .foregroundStyle(.tertiary)
                        .tip("Borrar la búsqueda")
                }
            }
            .padding(.horizontal, 16)
            .frame(height: 52)
            Divider()
            if results.isEmpty {
                ContentUnavailableView(query.isEmpty ? "Sin chats todavía" : "Nada coincide con «\(query)»",
                                       systemImage: query.isEmpty ? "bubble.left.and.bubble.right" : "magnifyingglass")
                    .frame(maxHeight: .infinity)
            } else {
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 2, pinnedViews: .sectionHeaders) {
                            ForEach(HistoryDay.groups(results), id: \.day) { group in
                                Section {
                                    ForEach(group.chats, id: \.id) { summary in
                                        row(summary).id(summary.id)
                                    }
                                } header: {
                                    Text(group.day.title)
                                        .font(.caption.weight(.semibold))
                                        .foregroundStyle(.secondary)
                                        .frame(maxWidth: .infinity, alignment: .leading)
                                        .padding(.horizontal, 10)
                                        .padding(.top, 10)
                                        .padding(.bottom, 4)
                                }
                            }
                        }
                        .padding(8)
                    }
                    .onChange(of: selected) { _, id in if let id { proxy.scrollTo(id, anchor: .center) } }
                }
            }
        }
        .frame(width: 560, height: 420)
        .onAppear { focused = true; refresh() }
        .onChange(of: query) { _, _ in refresh() }
        .onKeyPress(.downArrow) { move(1); return .handled }
        .onKeyPress(.upArrow) { move(-1); return .handled }
        .onExitCommand(perform: onClose)
    }

    private func row(_ summary: ChatSummary) -> some View {
        let isSelected = summary.id == selected
        return Button { onOpen(summary.id) } label: {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(summary.title)
                        .font(.body.weight(.medium))
                        .lineLimit(1)
                    if !summary.preview.isEmpty {
                        Text(summary.preview.replacingOccurrences(of: "\n", with: " "))
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                Spacer(minLength: 8)
                Text(HistoryDay.label(summary.updatedAt))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(isSelected ? AnyShapeStyle(.selection.opacity(0.35)) : AnyShapeStyle(.clear),
                        in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { if $0 { selected = summary.id } }
        .contextMenu {
            Button("Abrir") { onOpen(summary.id) }
            Divider()
            Button("Eliminar chat", role: .destructive) {
                chat.delete(summary.id)
                refresh()
            }
        }
    }

    private func refresh() {
        results = chat.search(query)
        if !results.contains(where: { $0.id == selected }) { selected = results.first?.id }
    }

    private var ordered: [ChatSummary] { HistoryDay.groups(results).flatMap(\.chats) }

    private func move(_ step: Int) {
        let list = ordered
        guard !list.isEmpty else { return }
        let index = list.firstIndex { $0.id == selected } ?? -step
        selected = list[min(max(index + step, 0), list.count - 1)].id
    }

    private func openSelected() {
        if let selected { onOpen(selected) }
    }
}

/// The history window: centred on the screen where Buddy is.
@MainActor
final class HistoryWindow {
    private var panel: KeyPanel?

    func show(chat: ChatController, on screen: NSScreen?, onOpen: @escaping (String) -> Void) {
        close()
        let view = HistorySearchView(chat: chat,
                                     onOpen: { [weak self] id in self?.close(); onOpen(id) },
                                     onClose: { [weak self] in self?.close() })
        let size = CGSize(width: 560, height: 420)
        let (panel, _) = KeyPanel.make(visibleSize: size, cornerRadius: 20, view: view)
        let area = (screen ?? NSScreen.main)?.visibleFrame ?? .zero
        let visible = NSRect(x: area.midX - size.width / 2, y: area.midY - size.height / 2 + area.height * 0.08,
                             width: size.width, height: size.height)
        panel.setFrame(Surface.windowFrame(for: visible), display: false)
        NSApp.activate()
        panel.makeKeyAndOrderFront(nil)
        self.panel = panel
        // A click outside closes it.
        monitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, let frame = self.frame, !frame.contains(NSEvent.mouseLocation) else { return }
                self.close()
            }
        }
    }

    private var monitor: Any?

    var frame: NSRect? { panel.map(Surface.visibleFrame(of:)) }

    func close() {
        panel?.orderOut(nil)
        panel = nil
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
    }
}
