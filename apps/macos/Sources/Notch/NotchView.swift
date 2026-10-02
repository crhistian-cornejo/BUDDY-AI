import AppKit
import SwiftUI

/// Sizes of the island in each mode (the controller hit-tests with the same numbers). A 4-pt grid: 24 pt sides,
/// 16 pt between blocks, 20 pt at the bottom.
enum NotchLayout {
    static let earWidth: CGFloat = 34
    static let side: CGFloat = 24
    static let noticeWidth: CGFloat = 420
    static let openWidth: CGFloat = 560
    static let tileHeight: CGFloat = 116
    static let playerHeight: CGFloat = 92
    static let dropHeight: CGFloat = 112

    /// Lines the command box shows (wrapped at ~50 characters, at most 6).
    static func commandLines(_ text: String) -> Int {
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false)
            .reduce(0) { $0 + max(1, Int(ceil(Double($1.count) / 50))) }
        return min(max(lines, 1), 6)
    }

    @MainActor
    static func size(_ model: NotchModel, notch: CGSize) -> CGSize {
        switch model.mode {
        case .idle:
            return model.activeSession == nil && model.focus?.running != true
                ? notch : CGSize(width: notch.width + 2 * earWidth, height: notch.height)
        case .notice:
            let extra: CGFloat
            if let notice = model.notice, notice.isApproval {
                extra = 120 + (notice.command.isEmpty ? 0 : 20 + CGFloat(commandLines(notice.command)) * 16)
            } else {
                extra = 80
            }
            return CGSize(width: max(noticeWidth, notch.width + 48), height: notch.height + extra)
        case .open:
            let player = model.nowPlaying == nil ? 0 : playerHeight + 16
            return CGSize(width: max(openWidth, notch.width + 48), height: notch.height + 16 + player + tileHeight + 20)
        case .drop:
            return CGSize(width: max(noticeWidth, notch.width + 48), height: notch.height + 16 + dropHeight + 20)
        }
    }
}

extension NotchModel {
    /// The session the ears show: one waiting for the user first, else one working.
    var activeSession: Session? {
        sessions.first { $0.state == "waiting" } ?? sessions.first { $0.state == "working" }
    }
}

/// What the island can ask the controller to do.
struct NotchActions {
    var answer: (String, Bool) -> Void
    var connect: () -> Void
    var media: (MediaAction) -> Void
    var focusStart: (UInt32) -> Void
    var focusStop: () -> Void
    var openShortcut: (Shortcut) -> Void
    var addShortcut: () -> Void
    var removeShortcut: (Shortcut) -> Void
    var giveToBuddy: () -> Void
    var share: () -> Void
    var copyPaths: () -> Void
    var clearDrop: () -> Void
    var drop: ([URL]) -> Void
}

/// The island: black like the hardware notch, white text. One notice at a time; hover opens the tools.
struct NotchView: View {
    let model: NotchModel
    let notch: CGSize
    let hooksConnected: Bool
    let actions: NotchActions

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        let size = NotchLayout.size(model, notch: notch)
        let open = model.mode != .idle
        ZStack(alignment: .top) {
            NotchShape(earRadius: open ? 14 : 8, bottomRadius: open ? 24 : notch.height / 2.4)
                .fill(.black)
            content
                .padding(.top, notch.height)
                .padding(.horizontal, NotchLayout.side)
                .opacity(open ? 1 : 0)
            if model.mode == .idle {
                ears.frame(height: notch.height)
            }
        }
        .frame(width: size.width, height: size.height)
        .tooltipHost()
        .environment(\.colorScheme, .dark)
        .animation(reduceMotion ? nil : .spring(response: 0.38, dampingFraction: 0.82), value: size)
        .animation(reduceMotion ? nil : .easeOut(duration: 0.16), value: model.mode)
        .onDrop(of: [.fileURL], isTargeted: Binding(get: { model.dragging }, set: { model.dragging = $0 })) { providers in
            Task { @MainActor in actions.drop(await Self.urls(from: providers)) }
            return true
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
    }

    @ViewBuilder
    private var content: some View {
        switch model.mode {
        case .notice:
            if let notice = model.notice {
                NoticeCard(notice: notice, onAnswer: actions.answer, onDismiss: { model.dismiss() })
            }
        case .open:
            VStack(spacing: 16) {
                if let playing = model.nowPlaying {
                    MusicPlayer(track: playing, readAt: model.nowPlayingAt, onMedia: actions.media)
                }
                HStack(spacing: 12) {
                    SessionsTile(sessions: model.sessions, connected: hooksConnected, onConnect: actions.connect)
                    FocusTile(focus: model.focus, onStart: actions.focusStart, onStop: actions.focusStop)
                    ShortcutsTile(shortcuts: model.shortcuts, onOpen: actions.openShortcut, onAdd: actions.addShortcut,
                                  onRemove: actions.removeShortcut)
                }
                .frame(height: NotchLayout.tileHeight, alignment: .top)
            }
            .padding(.top, 16)
        case .drop:
            DropPanel(files: model.dropped, dragging: model.dragging, actions: actions)
                .padding(.top, 16)
        case .idle:
            EmptyView()
        }
    }

    /// At rest: the agent working (mark + dot) and the focus countdown, beside the notch.
    @ViewBuilder
    private var ears: some View {
        let session = model.activeSession
        let focusing = model.focus?.running == true
        if session != nil || focusing {
            HStack {
                Group {
                    if let session {
                        ProviderMark(provider: session.agent == "codex" ? "codex" : "claude", size: 14)
                            .tip("\(session.agent == "codex" ? "Codex" : "Claude Code") · \(session.project)")
                    } else {
                        Image(systemName: "timer").font(.system(size: 12, weight: .semibold)).foregroundStyle(.orange)
                    }
                }
                .frame(width: NotchLayout.earWidth)
                Spacer()
                Group {
                    if let session {
                        StateDot(state: session.state)
                    } else if let focus = model.focus {
                        FocusCountdown(endsAt: focus.endsAt, compact: true)
                    }
                }
                .frame(width: NotchLayout.earWidth)
            }
        }
    }

    static func urls(from providers: [NSItemProvider]) async -> [URL] {
        var out: [URL] = []
        for provider in providers where provider.hasItemConformingToTypeIdentifier("public.file-url") {
            if let url = try? await provider.loadItem(forTypeIdentifier: "public.file-url") as? Data,
               let file = URL(dataRepresentation: url, relativeTo: nil) {
                out.append(file)
            }
        }
        return out
    }
}

// MARK: - Notices

/// A coloured dot for a session: working (pulsing green), waiting (orange), done (blue), error (red).
struct StateDot: View {
    let state: String
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var color: Color {
        switch state {
        case "working": return .green
        case "waiting": return .orange
        case "error": return .red
        default: return .blue
        }
    }

    var body: some View {
        Circle()
            .fill(color)
            .frame(width: 7, height: 7)
            .phaseAnimator([1.0, 0.35], trigger: state) { dot, phase in
                dot.opacity(state == "working" && !reduceMotion ? phase : 1)
            } animation: { _ in .easeInOut(duration: 0.9) }
    }
}

private struct NoticeCard: View {
    let notice: NotchModel.Notice
    var onAnswer: (String, Bool) -> Void
    var onDismiss: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top, spacing: 12) {
                Group {
                    if notice.agent == "buddy" {
                        AvatarView(size: 24)
                    } else {
                        ProviderMark(provider: notice.agent == "codex" ? "codex" : "claude", size: 18)
                    }
                }
                    .frame(width: 34, height: 34)
                    .background(.white.opacity(0.1), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
                VStack(alignment: .leading, spacing: 2) {
                    Text(notice.title)
                        .font(.system(size: 13, weight: .semibold))
                        .lineLimit(1)
                    Text(notice.detail)
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
                Spacer(minLength: 0)
            }
            if !notice.command.isEmpty {
                Text(notice.command)
                    .font(.system(size: 11, design: .monospaced))
                    .lineLimit(6)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    .textSelection(.enabled)
            }
            if case let .approval(requestID, canAllow) = notice.kind {
                HStack(spacing: 8) {
                    if !canAllow {
                        Text("Es demasiado largo para revisarlo aquí: respóndelo en la terminal.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("Rechazar") { onAnswer(requestID, false) }
                        .buttonStyle(IslandButtonStyle(prominent: false))
                        .tip("No permitirlo; \(notice.agentName) seguirá sin hacerlo")
                    if canAllow {
                        Button("Permitir") { onAnswer(requestID, true) }
                            .buttonStyle(IslandButtonStyle(prominent: true))
                            .tip("Permitir esta vez")
                    }
                }
            }
        }
        .padding(.top, 12)
        .padding(.bottom, 20)
        .contentShape(Rectangle())
        .onTapGesture { if !notice.isApproval { onDismiss() } }
    }
}

// MARK: - Music

/// What plays in Spotify or Music: artwork, title, artist, a progress bar with times and the controls.
private struct MusicPlayer: View {
    let track: NowPlaying
    let readAt: Date
    var onMedia: (MediaAction) -> Void

    var body: some View {
        HStack(spacing: 14) {
            artwork
                .frame(width: 64, height: 64)
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            VStack(alignment: .leading, spacing: 6) {
                VStack(alignment: .leading, spacing: 1) {
                    Text(track.title).font(.system(size: 14, weight: .semibold)).lineLimit(1)
                    Text(track.artist.isEmpty ? track.app : "\(track.artist) · \(track.app)")
                        .font(.system(size: 12)).foregroundStyle(.secondary).lineLimit(1)
                }
                progress
            }
            HStack(spacing: 4) {
                control("backward.fill", "Anterior", .previous, size: 14)
                control(track.status == .playing ? "pause.fill" : "play.fill",
                        track.status == .playing ? "Pausar" : "Reproducir", .playPause, size: 20)
                control("forward.fill", "Siguiente", .next, size: 14)
            }
        }
        .padding(14)
        .frame(height: NotchLayout.playerHeight)
        .background(.white.opacity(0.07), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
    }

    @ViewBuilder
    private var progress: some View {
        if let duration = track.durationMs, duration > 0, let position = track.positionMs {
            // Runs on from the last reading while it plays, redrawn once a second (only while the island is open).
            TimelineView(.periodic(from: .now, by: 1)) { context in
                let elapsed = track.status == .playing ? Int(context.date.timeIntervalSince(readAt) * 1000) : 0
                let now = min(position + elapsed, duration)
                HStack(spacing: 8) {
                    Text(Self.time(now)).monospacedDigit()
                    ProgressView(value: Double(now), total: Double(duration))
                        .progressViewStyle(.linear)
                        .tint(.white)
                    Text("-" + Self.time(duration - now)).monospacedDigit()
                }
                .font(.system(size: 10, weight: .medium))
                .foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder
    private var artwork: some View {
        if let data = track.artwork, let image = NSImage(data: data) {
            Image(nsImage: image).resizable().aspectRatio(contentMode: .fill)
        } else {
            ZStack {
                Color.white.opacity(0.1)
                Image(systemName: "music.note").font(.system(size: 20)).foregroundStyle(.secondary)
            }
        }
    }

    private func control(_ symbol: String, _ help: String, _ action: MediaAction, size: CGFloat) -> some View {
        Button { onMedia(action) } label: {
            Image(systemName: symbol)
                .font(.system(size: size))
                .frame(width: 32, height: 32)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .tip(help)
        .accessibilityLabel(help)
    }

    static func time(_ ms: Int) -> String {
        let s = max(ms, 0) / 1000
        return String(format: "%d:%02d", s / 60, s % 60)
    }
}

// MARK: - Tiles

/// A tile of the overview: title with its symbol, then its content.
private struct Tile<Content: View>: View {
    let title: String
    let symbol: String
    @ViewBuilder let content: () -> Content

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label(title, systemImage: symbol)
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(.secondary)
            content()
            Spacer(minLength: 0)
        }
        .padding(12)
        .frame(maxWidth: .infinity, minHeight: NotchLayout.tileHeight, maxHeight: NotchLayout.tileHeight, alignment: .topLeading)
        .background(.white.opacity(0.07), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .clipped()
    }
}

private struct SessionsTile: View {
    let sessions: [NotchModel.Session]
    let connected: Bool
    var onConnect: () -> Void

    var body: some View {
        Tile(title: "Agentes", symbol: "terminal") {
            if sessions.isEmpty {
                Text(connected ? "Sin sesiones abiertas" : "Claude Code y Codex")
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                if !connected {
                    Button("Conectar", action: onConnect)
                        .buttonStyle(IslandButtonStyle(prominent: true, compact: true))
                        .tip("Muestra qué cambia en su configuración antes de hacerlo")
                }
            } else {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(sessions.prefix(3)) { session in
                        HStack(spacing: 6) {
                            ProviderMark(provider: session.agent == "codex" ? "codex" : "claude", size: 11)
                            Text(session.project).font(.system(size: 12, weight: .medium)).lineLimit(1)
                            Spacer(minLength: 2)
                            StateDot(state: session.state)
                        }
                        .tip(Self.stateText(session.state))
                    }
                }
            }
        }
    }

    static func stateText(_ state: String) -> String {
        switch state {
        case "working": return "Trabajando"
        case "waiting": return "Esperando tu respuesta"
        case "error": return "Terminó con un error"
        default: return "Terminó"
        }
    }
}

private struct FocusTile: View {
    let focus: FocusStatus?
    var onStart: (UInt32) -> Void
    var onStop: () -> Void

    var body: some View {
        Tile(title: "Enfoque", symbol: "timer") {
            if let focus, focus.running {
                HStack(spacing: 10) {
                    FocusRing(focus: focus).frame(width: 46, height: 46)
                    Button("Parar", action: onStop)
                        .buttonStyle(IslandButtonStyle(prominent: false, compact: true))
                        .tip("Terminar el bloque de enfoque ahora")
                }
            } else {
                Text("Sin distracciones")
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                HStack(spacing: 6) {
                    Button("25 min") { onStart(25) }
                        .buttonStyle(IslandButtonStyle(prominent: true, compact: true))
                        .tip("Empezar 25 minutos de enfoque")
                    Button("50") { onStart(50) }
                        .buttonStyle(IslandButtonStyle(prominent: false, compact: true))
                        .tip("Empezar 50 minutos de enfoque")
                }
            }
        }
    }
}

/// The time left as a ring with the minutes inside, redrawn once a second while visible.
private struct FocusRing: View {
    let focus: FocusStatus

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            let total = Double(max(focus.endsAt - focus.startedAt, 1))
            let left = max(Double(focus.endsAt) - context.date.timeIntervalSince1970, 0)
            ZStack {
                Circle().stroke(.white.opacity(0.15), lineWidth: 4)
                Circle()
                    .trim(from: 0, to: left / total)
                    .stroke(.orange, style: StrokeStyle(lineWidth: 4, lineCap: .round))
                    .rotationEffect(.degrees(-90))
                FocusCountdown(endsAt: focus.endsAt, compact: false)
            }
        }
    }
}

struct FocusCountdown: View {
    let endsAt: Int64
    let compact: Bool

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            let left = max(Int(Double(endsAt) - context.date.timeIntervalSince1970), 0)
            Text(compact ? "\(Int(ceil(Double(left) / 60)))m" : String(format: "%d:%02d", left / 60, left % 60))
                .font(.system(size: compact ? 11 : 10, weight: .semibold))
                .monospacedDigit()
                .foregroundStyle(compact ? .orange : .white)
        }
    }
}

private struct ShortcutsTile: View {
    let shortcuts: [Shortcut]
    var onOpen: (Shortcut) -> Void
    var onAdd: () -> Void
    var onRemove: (Shortcut) -> Void

    private let columns = Array(repeating: GridItem(.fixed(30), spacing: 8), count: 4)

    var body: some View {
        Tile(title: "Atajos", symbol: "square.grid.2x2") {
            LazyVGrid(columns: columns, alignment: .leading, spacing: 8) {
                ForEach(shortcuts, id: \.id) { item in
                    Button { onOpen(item) } label: {
                        ShortcutIcon(item: item).frame(width: 30, height: 30)
                    }
                    .buttonStyle(.plain)
                    .tip(item.name)
                    .contextMenu {
                        Button("Quitar de atajos") { onRemove(item) }
                    }
                }
                if shortcuts.count < 8 {
                    Button(action: onAdd) {
                        Image(systemName: "plus")
                            .font(.system(size: 13, weight: .semibold))
                            .frame(width: 30, height: 30)
                            .background(.white.opacity(0.1), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                    }
                    .buttonStyle(.plain)
                    .tip("Fijar una app, carpeta o archivo")
                }
            }
        }
    }
}

private struct ShortcutIcon: View {
    let item: Shortcut

    var body: some View {
        if item.kind == "web" {
            Image(systemName: "globe")
                .font(.system(size: 15))
                .frame(width: 30, height: 30)
                .background(.white.opacity(0.1), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        } else {
            Image(nsImage: NSWorkspace.shared.icon(forFile: item.target))
                .resizable()
                .interpolation(.high)
        }
    }
}

// MARK: - Drop

/// «Suelta tus archivos aquí»: while dragging, a target; once dropped, what to do with them.
private struct DropPanel: View {
    let files: [URL]
    let dragging: Bool
    let actions: NotchActions

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if files.isEmpty {
                VStack(spacing: 8) {
                    Image(systemName: "tray.and.arrow.down")
                        .font(.system(size: 22, weight: .medium))
                        .symbolEffect(.bounce, value: dragging)
                    Text("Suelta tus archivos aquí").font(.system(size: 13, weight: .semibold))
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .strokeBorder(.white.opacity(0.35), style: StrokeStyle(lineWidth: 1.5, dash: [6, 5])))
            } else {
                HStack(spacing: 10) {
                    Image(nsImage: NSWorkspace.shared.icon(forFile: files[0].path))
                        .resizable().frame(width: 32, height: 32)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(files.count == 1 ? files[0].lastPathComponent : "\(files.count) archivos")
                            .font(.system(size: 13, weight: .semibold)).lineLimit(1)
                        Text(files.count == 1 ? files[0].deletingLastPathComponent().path : files.map(\.lastPathComponent).joined(separator: ", "))
                            .font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                    }
                    Spacer()
                    Button(action: actions.clearDrop) { Image(systemName: "xmark").frame(width: 24, height: 24) }
                        .buttonStyle(.plain)
                        .tip("Descartar")
                }
                HStack(spacing: 8) {
                    Button("Dárselo a Buddy", action: actions.giveToBuddy)
                        .buttonStyle(IslandButtonStyle(prominent: true))
                        .tip("Abre el chat con los archivos")
                    Button("Compartir…", action: actions.share)
                        .buttonStyle(IslandButtonStyle(prominent: false))
                        .tip("AirDrop, Mail, Mensajes…")
                    Button("Copiar ruta", action: actions.copyPaths)
                        .buttonStyle(IslandButtonStyle(prominent: false))
                        .tip("Copia la ruta al portapapeles")
                }
            }
        }
        .frame(height: NotchLayout.dropHeight, alignment: .top)
    }
}

/// Buttons on the black island: white for the main action, translucent for the other (like the Dynamic Island).
struct IslandButtonStyle: ButtonStyle {
    let prominent: Bool
    var compact = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: compact ? 12 : 13, weight: .semibold))
            .foregroundStyle(prominent ? Color.black : Color.white)
            .padding(.horizontal, compact ? 10 : 14)
            .frame(height: compact ? 24 : 28)
            .background(prominent ? Color.white : Color.white.opacity(0.16), in: Capsule())
            .opacity(configuration.isPressed ? 0.7 : 1)
            .contentShape(Capsule())
    }
}
