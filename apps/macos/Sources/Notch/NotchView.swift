import AppKit
import SwiftUI

/// Sizes of the island in each mode (the controller hit-tests with the same numbers). A 4-pt grid: 24 pt sides,
/// 16 pt between blocks, 20 pt at the bottom.
enum NotchLayout {
    static let earWidth: CGFloat = 40
    static let side: CGFloat = 24
    static let noticeWidth: CGFloat = 420
    static let openWidth: CGFloat = 560
    static let tileHeight: CGFloat = 116
    static let playerHeight: CGFloat = 92
    static let dropHeight: CGFloat = 112
    static let usageRow: CGFloat = 14
    static let briefingRow: CGFloat = 18
    static let briefingMax = 3
    /// The «Hoy» list: one line per mensajito, at most three.
    static func briefingHeight(_ count: Int) -> CGFloat {
        let rows = CGFloat(min(count, briefingMax))
        return rows * briefingRow + (rows - 1) * 6
    }
    /// The usage strip: each plan is a column, its windows (5 h, week…) rows under one another.
    @MainActor
    static func usageHeight(_ usage: [ProviderUsage]) -> CGFloat {
        let rows = CGFloat(min(usage.map { $0.windows.count }.max() ?? 1, 3))
        return rows * usageRow + (rows - 1) * 5
    }

    /// Lines the command box shows (wrapped at ~50 characters, at most 6).
    static func commandLines(_ text: String) -> Int {
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false)
            .reduce(0) { $0 + max(1, Int(ceil(Double($1.count) / 50))) }
        return min(max(lines, 1), 6)
    }

    @MainActor
    static func size(_ model: NotchModel, notch: CGSize, mode: NotchModel.Mode? = nil) -> CGSize {
        switch mode ?? model.mode {
        case .idle:
            return model.ear == .none ? notch : CGSize(width: notch.width + 2 * earWidth, height: notch.height)
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
            let usage = model.usage.isEmpty ? 0 : usageHeight(model.usage) + 12
            let news = model.briefing.isEmpty ? 0 : briefingHeight(model.briefing.count) + 12
            return CGSize(width: max(openWidth, notch.width + 48),
                          height: notch.height + 16 + player + tileHeight + news + usage + 20)
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

    /// What sits beside the notch at rest, by priority: Buddy answering, the focus
    /// countdown, the music playing.
    enum Ear: Equatable { case none, session(Session), buddy, focus(FocusStatus), music(EarTrack) }

    var ear: Ear {
        if buddyBusy { return .buddy }
        if let focus, focus.running { return .focus(focus) }
        if let track = earTrack, track.playing { return .music(track) }
        return .none
    }
}

/// What the island can ask the controller to do.
struct NotchActions {
    var answer: (String, Bool) -> Void
    var openPlace: (NotchModel.Place) -> Void
    var connect: () -> Void
    var media: (MediaAction) -> Void
    var seek: (Int) -> Void
    var focusStart: (UInt32) -> Void
    var focusStop: () -> Void
    var openShortcut: (Shortcut) -> Void
    var addShortcut: () -> Void
    var removeShortcut: (Shortcut) -> Void
    var openLink: (String) -> Void
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
    @State private var reveal: CGFloat = 0
    @State private var presentationSize: CGSize = .zero
    @State private var transitionID: UInt64 = 0
    @State private var expandedVisible = false
    @State private var retainedMode: NotchModel.Mode = .open
    @State private var retainedNotice: NotchModel.Notice?
    @State private var retainedSize: CGSize = .zero
    @State private var retainedDropped: [URL] = []

    private var motion: Animation? { reduceMotion ? nil : .spring(response: 0.38, dampingFraction: 0.88) }

    var body: some View {
        let open = model.mode != .idle
        let idleSize = NotchLayout.size(model, notch: notch, mode: .idle)
        let expandedSize = open ? NotchLayout.size(model, notch: notch) : retainedSize
        NotchMorph(progress: reveal, size: presentationSize == .zero ? idleSize : presentationSize,
                   idleSize: idleSize, expandedSize: expandedSize, notch: notch,
                   expanded: expandedContent, ears: ears)
        .tooltipHost()
        .environment(\.colorScheme, .dark)
        .onChange(of: model.mode, initial: true) { _, mode in
            transitionID &+= 1
            let transition = transitionID
            if mode != .idle {
                retainedMode = mode
                retainedNotice = model.notice
                retainedSize = NotchLayout.size(model, notch: notch)
                expandedVisible = true
            }
            withAnimation(motion, completionCriteria: .removed) {
                reveal = mode == .idle ? 0 : 1
                presentationSize = NotchLayout.size(model, notch: notch)
            } completion: {
                // An interrupted close must not remove the content of a newly reopened island.
                if transitionID == transition && model.mode == .idle {
                    expandedVisible = false
                    retainedNotice = nil
                }
            }
        }
        .onChange(of: NotchLayout.size(model, notch: notch)) { _, size in
            if open { retainedSize = size }
            withAnimation(motion) { presentationSize = size }
        }
        .onChange(of: model.notice) { _, notice in
            if let notice { retainedNotice = notice }
        }
        .onChange(of: model.dropped) { _, files in
            if !files.isEmpty { retainedDropped = files }
        }
        .onDrop(of: [.fileURL], isTargeted: Binding(get: { model.dragging }, set: { model.dragging = $0 })) { providers in
            Task { @MainActor in actions.drop(await Self.urls(from: providers)) }
            return true
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
    }

    @ViewBuilder
    private var expandedContent: some View {
        if expandedVisible { content.transition(.identity) }
    }

    @ViewBuilder
    private var content: some View {
        switch model.mode == .idle ? retainedMode : model.mode {
        case .notice:
            if let notice = model.notice ?? retainedNotice {
                NoticeCard(notice: notice, onAnswer: actions.answer, onDismiss: { model.dismiss() },
                           onOpen: { place in actions.openPlace(place); model.dismiss() })
            }
        case .open:
            VStack(spacing: 16) {
                if let playing = model.nowPlaying {
                    MusicPlayer(track: playing, readAt: model.nowPlayingAt, onMedia: actions.media, onSeek: actions.seek)
                }
                HStack(spacing: 12) {
                    SessionsTile(sessions: model.sessions, connected: hooksConnected, onConnect: actions.connect)
                    FocusTile(focus: model.focus, onStart: actions.focusStart, onStop: actions.focusStop)
                    ShortcutsTile(shortcuts: model.shortcuts, onOpen: actions.openShortcut, onAdd: actions.addShortcut,
                                  onRemove: actions.removeShortcut)
                }
                .frame(height: NotchLayout.tileHeight, alignment: .top)
                if !model.briefing.isEmpty {
                    BriefingList(items: model.briefing, onOpen: actions.openLink)
                        .padding(.top, -4)
                }
                if !model.usage.isEmpty {
                    UsageStrip(usage: model.usage)
                        .padding(.top, -4)
                }
            }
            .padding(.top, 16)
        case .drop:
            DropPanel(files: model.mode == .idle ? retainedDropped : model.dropped, dragging: model.dragging, actions: actions)
                .padding(.top, 16)
        case .idle:
            EmptyView()
        }
    }

    /// At rest, beside the notch: whatever matters most right now (see `NotchModel.ear`).
    @ViewBuilder
    private var ears: some View {
        switch model.ear {
        case .none:
            EmptyView()
        case let .session(session):
            earPair(left: AnyView(ProviderMark(provider: AgentNames.mark(session.agent), size: 14)),
                    right: AnyView(StateDot(state: session.state)),
                    tip: "\(AgentNames.name(session.agent)) · \(session.project) · \(SessionsTile.stateText(session.state))")
        case .buddy:
            earPair(left: AnyView(AvatarView(size: 18)),
                    right: AnyView(Group {
                        if let activity = model.buddyActivity { ActivityGlyph(kind: activity.kind) } else { ThinkingDots() }
                    }),
                    tip: model.buddyActivity?.label ?? "Buddy está respondiendo")
        case let .focus(focus):
            earPair(left: AnyView(Image(systemName: "timer").font(.system(size: 12, weight: .semibold)).foregroundStyle(Color.buddyIndigo)),
                    right: AnyView(FocusCountdown(endsAt: focus.endsAt, compact: true)),
                    tip: "Enfoque")
        case let .music(track):
            earPair(left: AnyView(Image(systemName: "music.note").font(.system(size: 12, weight: .semibold)).foregroundStyle(Color.accentColor)),
                    right: AnyView(Equalizer()),
                    tip: "\(track.title) · \(track.artist)")
        }
    }

    private func earPair(left: AnyView, right: AnyView, tip: String) -> some View {
        HStack {
            left.frame(width: NotchLayout.earWidth - 8)
            Spacer()
            right.frame(width: NotchLayout.earWidth - 8)
        }
        .padding(.horizontal, 8)
        .contentShape(Rectangle())
        .tip(tip)
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

/// One spring interpolates the background size and content opacity together. The expanded layout stays fixed while
/// the outline shrinks, avoiding text reflow and independent insertion/removal fades during the spring.
private struct NotchMorph<Expanded: View, Ears: View>: View, Animatable {
    var progress: CGFloat
    var size: CGSize
    let idleSize: CGSize
    let expandedSize: CGSize
    let notch: CGSize
    let expanded: Expanded
    let ears: Ears

    nonisolated var animatableData: AnimatablePair<CGFloat, AnimatablePair<CGFloat, CGFloat>> {
        get { .init(progress, .init(size.width, size.height)) }
        set { progress = newValue.first; size = CGSize(width: newValue.second.first, height: newValue.second.second) }
    }

    var body: some View {
        let p = min(max(progress, 0), 1)
        let width = max(size.width, 1)
        let height = max(size.height, 1)
        let shape = NotchShape(earRadius: 8 + 6 * p, bottomRadius: notch.height / 2.4 + (24 - notch.height / 2.4) * p)
        ZStack(alignment: .top) {
            expanded
                .frame(width: max(expandedSize.width - 2 * NotchLayout.side, 1), alignment: .top)
                .padding(.top, notch.height)
                .opacity(p)
                .scaleEffect(0.96 + 0.04 * p, anchor: .top)
                .offset(y: -6 * (1 - p))
                .allowsHitTesting(p == 1)
                .accessibilityHidden(p < 1)
            if p < 1 {
                ears.frame(width: idleSize.width, height: notch.height)
                    .opacity(1 - p)
                    .allowsHitTesting(p == 0)
            }
        }
        .frame(width: width, height: height, alignment: .top)
        .background(shape.fill(.black))
        .clipShape(shape)
        // Geometry and opacity are already interpolated together; descendants must not start a second fade.
        .animation(nil, value: progress)
        .animation(nil, value: size)
    }
}

/// Three bars moving like a level meter while music plays (still with reduced motion).
private struct Equalizer: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        HStack(alignment: .bottom, spacing: 2) {
            ForEach(0..<3) { i in
                Capsule()
                    .fill(Color.accentColor)
                    .frame(width: 3)
                    .phaseAnimator(reduceMotion ? [0.6] : [0.35, 1.0, 0.55, 0.85]) { bar, phase in
                        bar.frame(height: 12 * (i == 1 ? phase : 1.35 - phase))
                    } animation: { _ in .easeInOut(duration: 0.32 + Double(i) * 0.07) }
            }
        }
        .frame(height: 12)
    }
}

/// What Buddy is doing, as an animated symbol in the app's colours (Word blue, Excel green, PowerPoint orange…).
/// Still with reduced motion.
private struct ActivityGlyph: View {
    let kind: String
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var symbol: (name: String, color: Color) {
        switch kind {
        case "word": ("doc.text.fill", Color(red: 0.17, green: 0.38, blue: 0.80))
        case "excel": ("tablecells.fill", Color(red: 0.13, green: 0.55, blue: 0.33))
        case "powerpoint": ("rectangle.on.rectangle.angled.fill", Color(red: 0.86, green: 0.36, blue: 0.20))
        case "web": ("globe", .accentColor)
        case "read": ("doc.text.magnifyingglass", .white)
        case "command": ("apple.terminal.fill", .white)
        case "edit": ("square.and.pencil", .white)
        case "screen": ("eye.fill", Color.buddyIndigo)
        case "music": ("music.note", .accentColor)
        case "skill": ("sparkles", Color.buddyIndigo)
        default: ("wrench.and.screwdriver.fill", .white)
        }
    }

    var body: some View {
        let s = symbol
        Image(systemName: s.name)
            .font(.system(size: 12, weight: .semibold))
            .foregroundStyle(s.color)
            .symbolEffect(.pulse, options: .repeating, isActive: !reduceMotion && kind != "web")
            .symbolEffect(.rotate, options: .repeating, isActive: !reduceMotion && kind == "web")
            .contentTransition(.symbolEffect(.replace))
            .accessibilityLabel(kind)
    }
}

/// Three dots that breathe while Buddy writes.
private struct ThinkingDots: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        HStack(spacing: 3) {
            ForEach(0..<3) { i in
                Circle()
                    .fill(.white)
                    .frame(width: 4, height: 4)
                    .phaseAnimator(reduceMotion ? [1.0] : [0.3, 1.0]) { dot, phase in dot.opacity(phase) }
                        animation: { _ in .easeInOut(duration: 0.5).delay(Double(i) * 0.15) }
            }
        }
    }
}

// MARK: - Notices

/// A coloured dot for a session: working (pulsing green), waiting (orange), done (blue), error (red).
struct StateDot: View {
    let state: String
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var color: Color {
        switch state {
        case "working": return .accentColor
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
    var onOpen: (NotchModel.Place) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top, spacing: 12) {
                Group {
                    if notice.agent == "buddy" {
                        AvatarView(size: 24)
                    } else {
                        ProviderMark(provider: AgentNames.mark(notice.agent), size: 18)
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
        .onTapGesture {
            guard !notice.isApproval else { return }
            if let place = notice.place { onOpen(place) } else { onDismiss() }
        }
        .tip(notice.place == nil ? "" : "Volver a \(notice.agentName)")
    }
}

// MARK: - Music

/// What plays in Spotify or Music: artwork, title, artist, a progress bar with times and the controls.
private struct MusicPlayer: View {
    let track: NowPlaying
    let readAt: Date
    var onMedia: (MediaAction) -> Void
    var onSeek: (Int) -> Void
    /// While the bar is being dragged: where it would go.
    @State private var scrub: Double?

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
                let now = scrub.map { Int($0 * Double(duration)) } ?? min(position + elapsed, duration)
                HStack(spacing: 8) {
                    Text(Self.time(now)).monospacedDigit()
                    SeekBar(fraction: Double(now) / Double(duration), scrubbing: $scrub) { fraction in
                        onSeek(Int(fraction * Double(duration)))
                    }
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

/// The track's progress: tap or drag to move through the song.
private struct SeekBar: View {
    let fraction: Double
    @Binding var scrubbing: Double?
    var onSeek: (Double) -> Void
    @State private var hovering = false

    var body: some View {
        GeometryReader { geo in
            let width = max(geo.size.width, 1)
            ZStack(alignment: .leading) {
                Capsule().fill(.white.opacity(0.18))
                Capsule().fill(.white).frame(width: width * min(max(fraction, 0), 1))
            }
            .frame(height: hovering || scrubbing != nil ? 6 : 4)
            .frame(maxHeight: .infinity)
            .contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 0)
                .onChanged { scrubbing = min(max($0.location.x / width, 0), 1) }
                .onEnded { value in
                    let f = min(max(value.location.x / width, 0), 1)
                    onSeek(f)
                    // Keep the dragged spot until the player reports the new position.
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) { scrubbing = nil }
                })
            .onHover { hovering = $0 }
        }
        .frame(height: 14)
        .animation(.easeOut(duration: 0.12), value: hovering)
        .tip("Toca o arrastra para avanzar")
    }
}

// MARK: - Usage

/// What is used of each plan: one column per provider (its mark on the left), a row per window under one another, so
/// every row is plainly that provider's.
private struct UsageStrip: View {
    let usage: [ProviderUsage]

    var body: some View {
        HStack(alignment: .top, spacing: 18) {
            ForEach(usage, id: \.provider) { plan in
                HStack(alignment: .top, spacing: 8) {
                    ProviderMark(provider: plan.provider, size: 12)
                        .frame(height: NotchLayout.usageRow)
                    VStack(alignment: .leading, spacing: 5) {
                        ForEach(plan.windows.prefix(3), id: \.label) { window in
                            UsageBar(window: window)
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .frame(height: NotchLayout.usageHeight(usage), alignment: .top)
    }
}

/// Today's «mensajitos»: the topic, then the line; a click opens its source.
private struct BriefingList: View {
    let items: [BriefingItem]
    var onOpen: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(Array(items.prefix(NotchLayout.briefingMax).enumerated()), id: \.offset) { _, item in
                Button {
                    if let url = item.url { onOpen(url) }
                } label: {
                    HStack(spacing: 8) {
                        Text(item.topic)
                            .font(.system(size: 10, weight: .semibold))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .frame(width: 72, alignment: .leading)
                        Text(item.text)
                            .font(.system(size: 12))
                            .lineLimit(1)
                            .truncationMode(.tail)
                        Spacer(minLength: 0)
                        if item.url != nil {
                            Image(systemName: "arrow.up.right")
                                .font(.system(size: 9, weight: .semibold))
                                .foregroundStyle(.tertiary)
                        }
                    }
                    .frame(height: NotchLayout.briefingRow)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(item.url == nil)
                .tip(item.text)
            }
        }
        .frame(height: NotchLayout.briefingHeight(items.count), alignment: .top)
    }
}

private struct UsageBar: View {
    let window: UsageWindow

    private var color: Color { window.usedPct >= 90 ? .red : window.usedPct >= 70 ? .orange : .white }

    var body: some View {
        HStack(spacing: 6) {
            Text(window.label)
                .foregroundStyle(.secondary)
                .frame(width: 44, alignment: .leading)
            ZStack(alignment: .leading) {
                Capsule().fill(.white.opacity(0.15))
                GeometryReader { geo in
                    Capsule().fill(color).frame(width: geo.size.width * min(max(window.usedPct / 100, 0), 1))
                }
            }
            .frame(height: 3)
            Text("\(Int(window.usedPct.rounded())) %")
                .monospacedDigit()
                .frame(width: 34, alignment: .trailing)
        }
        .font(.system(size: 10, weight: .semibold))
        .frame(height: NotchLayout.usageRow)
        .tip(Self.help(window))
    }

    static func help(_ w: UsageWindow) -> String {
        var text = "\(Int(w.usedPct.rounded())) % usado (\(w.label))"
        if let resets = w.resetsAt {
            let date = Date(timeIntervalSince1970: TimeInterval(resets))
            let f = DateFormatter()
            f.locale = Locale(identifier: "es")
            f.setLocalizedDateFormatFromTemplate(Calendar.current.isDateInToday(date) ? "HH:mm" : "EEE d HH:mm")
            text += " · se reinicia \(f.string(from: date))"
        }
        return text
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

struct SessionsTile: View {
    let sessions: [NotchModel.Session]
    let connected: Bool
    var onConnect: () -> Void

    var body: some View {
        Tile(title: "Agentes", symbol: "terminal") {
            if sessions.isEmpty {
                Text(connected ? "Sin sesiones abiertas" : "Claude, Codex y Gemini")
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
                            ProviderMark(provider: AgentNames.mark(session.agent), size: 11)
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
                    .stroke(Color.buddyIndigo, style: StrokeStyle(lineWidth: 4, lineCap: .round))
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
                .foregroundStyle(compact ? Color.buddyIndigo : .white)
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
