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
    /// The highlighted row (pointer or arrows).
    @State private var selected: String?
    /// Set only by the arrow keys: scrolling after a hover would move the rows under the pointer and loop.
    @State private var keyboardTarget: String?
    /// Chats picked to be deleted together (⌘-click, ⇧-click or «Seleccionar»).
    @State private var marked: Set<String> = []
    /// Where a ⇧-click range starts.
    @State private var anchor: String?
    /// «Seleccionar» is on: every row shows its checkbox and a click marks it instead of opening it.
    @State private var selecting = false
    /// The pointer is on the highlighted row's trash (it turns red).
    @State private var trashHovered = false
    @FocusState private var focused: Bool

    /// Clicks mark rows instead of opening them.
    private var picking: Bool { selecting || !marked.isEmpty }

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
                    .onSubmit(submit)
                if !query.isEmpty {
                    Button { query = "" } label: { Image(systemName: "xmark.circle.fill") }
                        .buttonStyle(.borderless)
                        .foregroundStyle(.tertiary)
                        .tip("Borrar la búsqueda")
                }
                if selecting || !results.isEmpty {
                    Button(selecting ? "Listo" : "Seleccionar") { toggleSelecting() }
                        .buttonStyle(.borderless)
                        .tip(selecting ? "Dejar de seleccionar" : "Elegir varios chats para borrarlos")
                        .accessibilityAddTraits(selecting ? .isSelected : [])
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
                        LazyVStack(alignment: .leading, spacing: 2) {
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
                                        .accessibilityAddTraits(.isHeader)
                                }
                            }
                        }
                        .padding(8)
                    }
                    .onChange(of: keyboardTarget) { _, id in if let id { proxy.scrollTo(id) } }
                }
            }
            if !marked.isEmpty { selectionBar }
        }
        .frame(width: 560, height: 420)
        // ⌘⌫ deletes what is marked, or the highlighted chat (always after asking).
        .background {
            Button("Borrar", action: deleteRequested)
                .keyboardShortcut(.delete, modifiers: .command)
                .opacity(0)
                .frame(width: 0, height: 0)
                .accessibilityHidden(true)
        }
        .onAppear { focused = true; refresh() }
        .onChange(of: query) { _, _ in refresh() }
        // The trash goes away with its row before it can see the pointer leave.
        .onChange(of: selected) { _, _ in trashHovered = false }
        .onKeyPress(.downArrow) { move(1); return .handled }
        .onKeyPress(.upArrow) { move(-1); return .handled }
        // ⌫ / ⌦ only when there is no search text to erase.
        .onKeyPress(keys: [.delete, .deleteForward], phases: .down) { _ in
            guard query.isEmpty, !results.isEmpty else { return .ignored }
            deleteRequested()
            return .handled
        }
        .onExitCommand { picking ? clearSelection() : onClose() }
    }

    private var selectionBar: some View {
        VStack(spacing: 0) {
            Divider()
            HStack(spacing: 10) {
                Text(marked.count == 1 ? "1 seleccionado" : "\(marked.count) seleccionados")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
                Spacer()
                Button("Cancelar", action: clearSelection)
                    .tip("Quitar la selección", shortcut: "esc")
                Button("Borrar", role: .destructive) { confirmDelete(Array(marked)) }
                    .buttonStyle(.borderedProminent)
                    .tint(.red)
                    .tip(marked.count == 1 ? "Borrar el chat seleccionado" : "Borrar los chats seleccionados", shortcut: "⌘⌫")
            }
            .controlSize(.regular)
            .padding(.horizontal, 16)
            .frame(height: 48)
        }
    }

    private func row(_ summary: ChatSummary) -> some View {
        let isHighlighted = summary.id == selected
        let isMarked = marked.contains(summary.id)
        let showTrash = isHighlighted && !picking
        return Button { click(summary.id) } label: {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                if picking {
                    Image(systemName: isMarked ? "checkmark.circle.fill" : "circle")
                        .font(.system(size: 15))
                        .foregroundStyle(isMarked ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
                        .accessibilityHidden(true)
                }
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
                    // The trash takes its place under the pointer.
                    .opacity(showTrash ? 0 : 1)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(background(highlighted: isHighlighted, marked: isMarked),
                        in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isMarked ? .isSelected : [])
        .accessibilityHint(picking ? "Marca o desmarca este chat" : "Abre el chat")
        .accessibilityAction(named: "Borrar chat") { confirmDelete([summary.id]) }
        .overlay(alignment: .trailing) {
            if showTrash {
                Button { confirmDelete([summary.id]) } label: {
                    Image(systemName: "trash")
                        .font(.system(size: 13, weight: .medium))
                        .foregroundStyle(trashHovered ? AnyShapeStyle(.red) : AnyShapeStyle(.secondary))
                        .frame(width: 26, height: 26)
                        .background(trashHovered ? Color.red.opacity(0.14) : .clear,
                                    in: RoundedRectangle(cornerRadius: 6, style: .continuous))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.borderless)
                .onHover { trashHovered = $0 }
                .tip("Borrar chat", iconOnly: true)
                .padding(.trailing, 6)
            }
        }
        .onHover { if $0 { selected = summary.id } }
        .contextMenu {
            Button("Abrir") { onOpen(summary.id) }
            Button(isMarked ? "Quitar de la selección" : "Seleccionar") { toggle(summary.id) }
            Divider()
            if isMarked && marked.count > 1 {
                Button("Borrar \(marked.count) chats", role: .destructive) { confirmDelete(Array(marked)) }
            }
            Button("Borrar chat", role: .destructive) { confirmDelete([summary.id]) }
        }
    }

    private func background(highlighted: Bool, marked: Bool) -> AnyShapeStyle {
        switch (highlighted, marked) {
        case (_, true): return AnyShapeStyle(Color.accentColor.opacity(highlighted ? 0.26 : 0.18))
        case (true, false): return AnyShapeStyle(.selection.opacity(0.35))
        case (false, false): return AnyShapeStyle(.clear)
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
        let next = list[min(max(index + step, 0), list.count - 1)].id
        selected = next
        keyboardTarget = next
    }

    /// Return opens the highlighted chat, or marks it while picking.
    private func submit() {
        guard let selected else { return }
        picking ? toggle(selected) : onOpen(selected)
    }

    // MARK: Selecting

    /// A click opens the chat; ⌘-click marks or unmarks it, ⇧-click marks the range from the last one.
    private func click(_ id: String) {
        let flags = NSEvent.modifierFlags.intersection(.deviceIndependentFlagsMask)
        if flags.contains(.shift) { extend(to: id) }
        else if flags.contains(.command) || picking { toggle(id) }
        else { onOpen(id) }
    }

    private func toggle(_ id: String) {
        if marked.remove(id) == nil { marked.insert(id) }
        anchor = id
    }

    private func extend(to id: String) {
        let list = ordered.map(\.id)
        guard let end = list.firstIndex(of: id) else { return }
        let start = anchor.flatMap { list.firstIndex(of: $0) } ?? end
        marked.formUnion(list[min(start, end)...max(start, end)])
        if anchor == nil { anchor = id }
    }

    private func toggleSelecting() {
        selecting.toggle()
        if !selecting { clearSelection() }
    }

    private func clearSelection() {
        marked = []
        anchor = nil
        selecting = false
    }

    // MARK: Deleting

    /// What ⌘⌫ / ⌫ delete: the marked chats, or else the highlighted one.
    private func deleteRequested() {
        if !marked.isEmpty { confirmDelete(Array(marked)) }
        else if let selected { confirmDelete([selected]) }
    }

    /// Always asks first; nothing is deleted without «Borrar».
    private func confirmDelete(_ ids: [String]) {
        guard !ids.isEmpty else { return }
        let title = ids.count == 1 ? results.first { $0.id == ids[0] }?.title : nil
        // Out of the click or key handler: a modal run loop inside SwiftUI's event handling is asking for trouble.
        DispatchQueue.main.async {
            let alert = NSAlert()
            alert.alertStyle = .warning
            alert.messageText = ids.count == 1 ? "¿Borrar 1 chat?" : "¿Borrar \(ids.count) chats?"
            alert.informativeText = title.map { "«\($0)». No se puede deshacer." } ?? "No se puede deshacer."
            // Deleting needs a click on «Borrar»: no key confirms it (a stray Return must never delete), Esc cancels.
            let confirm = alert.addButton(withTitle: "Borrar")
            confirm.hasDestructiveAction = true
            confirm.keyEquivalent = ""
            alert.addButton(withTitle: "Cancelar").keyEquivalent = "\u{1b}"
            let window = NSApp.keyWindow
            NSApp.activate()
            let answer = alert.runModal()
            window?.makeKeyAndOrderFront(nil)
            focused = true
            guard answer == .alertFirstButtonReturn else { return }
            delete(ids)
        }
    }

    /// Deletes in the core (the chat on screen starts over if it was one of them) and reloads the list.
    private func delete(_ ids: [String]) {
        for id in ids { chat.delete(id) }
        marked.subtract(ids)
        if let anchor, ids.contains(anchor) { self.anchor = nil }
        refresh()
        if results.isEmpty { clearSelection() }
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
