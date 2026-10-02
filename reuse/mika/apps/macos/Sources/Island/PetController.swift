import AppKit
import SwiftUI
import Combine

// "Burbuja junto al notch": the circle beside the notch is MIKA's pet (twin of apps/windows/src/pet/main.ts).
// On Windows hovering it opens a ring with the other agents (here it does not: `ringOnHover`); a click (on it or on an agent) opens a small chat right there, the
// same chat as in the island (the AgentChat objects are shared, so both stay in step). While something plays in Spotify
// or Music the ring also has a "Música" item: it opens a small panel with the media strip (cover placeholder, title,
// previous / play-pause / next, the player's speaker and volume). Twin of the Windows pet's music panel, with one
// difference: the Mac has no events for "something is playing", so nothing runs while the circle is idle (no note
// badge on the circle); the players are asked only while the ring or the panel is open (AppState.petMediaOpen).

@MainActor
final class PetController {
    private let state: AppState
    private var ringPanel: NSPanel?
    private var chatPanel: PetChatPanel?
    private var hoverWork: DispatchWorkItem?
    private var leaveWork: DispatchWorkItem?
    /// When the ring opened: a panel appearing (or a frame change) must not count as the cursor leaving.
    private var ringOpenedAt: Date?
    /// The circle's hover area at the last frame, to check again before the ring closes.
    private var lastCircle: CGRect?
    private var keyMonitor: Any?
    private var mediaPanel: NSPanel?
    private var mediaIdleWork: DispatchWorkItem?
    private var mediaMonitor: Any?
    private var cancellables = Set<AnyCancellable>()

    static let chatSize = CGSize(width: 420, height: 360)
    /// The music panel stays open until this long without the mouse or keys touching it (never closes on mouse-leave).
    static let mediaIdle: TimeInterval = 12
    static let mediaSize = CGSize(width: 400, height: MediaStripMetrics.height + 20)

    /// The ring of agents on hover is the Windows pet's; on the Mac the circle only gives data (a click still opens the chat).
    static let ringOnHover = false

    var screen: () -> NSScreen? = { NSScreen.main }

    init(state: AppState) {
        self.state = state
        // The panel goes away when playback ends or the setting is turned off, and when the island itself opens.
        state.$nowPlaying.combineLatest(state.$mediaControl)
            .sink { [weak self] playing, enabled in
                if !enabled || playing?.isVisible != true { self?.closeMedia() }
            }
            .store(in: &cancellables)
        state.$mode.removeDuplicates()
            .sink { [weak self] mode in if mode != .hidden { self?.closeMedia() } }
            .store(in: &cancellables)
    }

    var isChatOpen: Bool { chatPanel != nil }
    var isRingOpen: Bool { ringPanel != nil }
    var isMediaOpen: Bool { mediaPanel != nil }

    /// A leave that comes this soon after the ring appeared is ignored (the Windows pet opened and closed its ring in a
    /// loop because a resize fired a stray mouse-exit; here the pointer is polled, but the same guard keeps it calm).
    static let settleAfterOpen: TimeInterval = 0.45

    /// Called every frame while the island is hidden. `circle` is the circle's hover area and `center` its centre, in
    /// screen coordinates; both nil when the circle is not out (the ring then closes).
    func track(mouse: CGPoint, circle: CGRect?, center: CGPoint?) {
        guard let circle, let center else {
            hoverWork?.cancel(); hoverWork = nil
            closeRing()
            return
        }
        let overCircle = circle.insetBy(dx: -4, dy: -4).contains(mouse)
        lastCircle = circle
        if let ringPanel {
            let settling = ringOpenedAt.map { Date().timeIntervalSince($0) < Self.settleAfterOpen } ?? false
            if overCircle || ringPanel.frame.contains(mouse) || settling {
                leaveWork?.cancel(); leaveWork = nil
            } else if leaveWork == nil {
                let work = DispatchWorkItem { [weak self] in
                    guard let self else { return }
                    self.leaveWork = nil
                    // Only if the pointer is really outside both, right now.
                    let now = NSEvent.mouseLocation
                    if let panel = self.ringPanel, panel.frame.contains(now)
                        || (self.lastCircle?.insetBy(dx: -4, dy: -4).contains(now) ?? false) { return }
                    self.closeRing()
                }
                leaveWork = work
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.32, execute: work)
            }
        } else if Self.ringOnHover, overCircle, hoverWork == nil, chatPanel == nil, mediaPanel == nil {
            let work = DispatchWorkItem { [weak self] in
                self?.hoverWork = nil
                self?.openRing(center: center)
            }
            hoverWork = work
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.26, execute: work)
        } else if !overCircle {
            hoverWork?.cancel(); hoverWork = nil
        }
    }

    // MARK: Ring

    private func openRing(center: CGPoint) {
        guard ringPanel == nil, chatPanel == nil, mediaPanel == nil, let screenFrame = screen()?.frame else { return }
        let others = state.hub.agents.filter { $0.id != Handoff.orchestratorID }
        guard !others.isEmpty else { return }
        // The window and the circle's place in it come from the pure `PetRing.macFrame`; the items from `PetRing.layout`
        // (inside the view, because "Música" can appear a moment after the ring opens).
        let placed = PetRing.macFrame(circle: center, screen: screenFrame)

        let panel = Self.makePanel(frame: NSRect(x: placed.window.minX, y: placed.window.minY,
                                                 width: placed.window.width, height: placed.window.height))
        panel.contentView = NSHostingView(rootView: PetRingView(
            state: state, agents: others, frame: placed.ring,
            onPick: { [weak self] id in self?.openChat(agentID: id) },
            onCircle: { [weak self] in self?.openChat(agentID: nil) },
            onMusic: { [weak self] in self?.openMedia() },
            onSettings: { [weak self] in
                self?.closeRing()
                NotificationCenter.default.post(name: .openFullSettings, object: nil)
            }))
        panel.orderFrontRegardless()
        ringPanel = panel
        ringOpenedAt = Date()
        state.setPetMediaOpen(true)             // the players are asked while the ring is open, so "Música" can appear
    }

    func closeRing() {
        leaveWork?.cancel(); leaveWork = nil
        ringOpenedAt = nil
        ringPanel?.orderOut(nil)
        ringPanel = nil
        if mediaPanel == nil { state.setPetMediaOpen(false) }
    }

    /// A click on the circle: closes the music panel when it is open (as on Windows), otherwise opens the chat.
    func circleTapped() {
        if mediaPanel != nil { closeMedia() } else { openChat(agentID: nil) }
    }

    // MARK: Music

    /// Opens the media strip in a small panel right under the circle. It closes by itself only after `mediaIdle` seconds
    /// without the mouse or keys touching it, on Escape, on a click on the circle, or when playback ends: never because
    /// the pointer left it.
    func openMedia() {
        guard mediaPanel == nil, state.mediaControl, state.nowPlaying?.isVisible == true,
              let screenFrame = screen()?.frame else { return }
        let size = Self.mediaSize
        let anchorX = (state.notchWidth > 0 ? screenFrame.midX + state.notchWidth / 2 : screenFrame.midX)
        let x = min(max(screenFrame.minX + 8, anchorX - 40), screenFrame.maxX - size.width - 8)
        let frame = NSRect(x: x, y: screenFrame.maxY - state.notchHeight - 8 - size.height, width: size.width, height: size.height)
        let panel = PetChatPanel(contentRect: frame, styleMask: [.borderless, .nonactivatingPanel],
                                 backing: .buffered, defer: false)
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = false
        panel.acceptsMouseMovedEvents = true
        panel.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.mainMenuWindow)) + 3)
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        panel.contentView = NSHostingView(rootView: PetMediaView(state: state, onClose: { [weak self] in self?.closeMedia() }))
        mediaPanel = panel                      // set before the ring closes, so the players keep being asked
        state.setPetMediaOpen(true)
        closeRing()
        panel.orderFrontRegardless()
        armMediaIdle()
        installKeyMonitor()
        // Any pointer or key activity on the panel restarts the idle timer.
        mediaMonitor = NSEvent.addLocalMonitorForEvents(
            matching: [.mouseMoved, .leftMouseDown, .leftMouseUp, .leftMouseDragged, .scrollWheel, .keyDown]) { [weak self] event in
            MainActor.assumeIsolated {
                if let self, let panel = self.mediaPanel, event.window === panel { self.armMediaIdle() }
            }
            return event
        }
    }

    func closeMedia() {
        mediaIdleWork?.cancel(); mediaIdleWork = nil
        guard let panel = mediaPanel else { return }
        panel.orderOut(nil)
        mediaPanel = nil
        if let mediaMonitor { NSEvent.removeMonitor(mediaMonitor) }
        mediaMonitor = nil
        removeKeyMonitorIfIdle()
        if ringPanel == nil { state.setPetMediaOpen(false) }
    }

    /// (Re)starts the 12 s idle timer; a button held down (a slider drag) postpones it.
    private func armMediaIdle() {
        mediaIdleWork?.cancel()
        let work = DispatchWorkItem { [weak self] in
            guard let self, self.mediaPanel != nil else { return }
            if NSEvent.pressedMouseButtons != 0 { self.armMediaIdle(); return }
            self.closeMedia()
        }
        mediaIdleWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.mediaIdle, execute: work)
    }

    // MARK: Chat

    /// Opens the chat of an agent (nil: the active one, MIKA unless the user chose another) right under the circle.
    func openChat(agentID: String?) {
        closeMedia()
        closeRing()
        let id = agentID ?? state.hub.activeID
        state.hub.select(id)
        state.setFocus(AgentTask.agentTaskID(id))
        guard let screenFrame = screen()?.frame else { return }
        let size = Self.chatSize
        let anchorX = (state.notchWidth > 0 ? screenFrame.midX + state.notchWidth / 2 : screenFrame.midX)
        let x = min(max(screenFrame.minX + 8, anchorX - 40), screenFrame.maxX - size.width - 8)
        // The panel starts `SatelliteLayout.chatGap` under the circle's body (not under the strip it sits in).
        let top = SatelliteLayout(notchHeight: state.notchHeight).chatTop
        let frame = NSRect(x: x, y: screenFrame.maxY - top - size.height, width: size.width, height: size.height)
        if chatPanel == nil {
            let panel = PetChatPanel(contentRect: frame, styleMask: [.borderless, .nonactivatingPanel],
                                     backing: .buffered, defer: false)
            panel.backgroundColor = .clear
            panel.isOpaque = false
            panel.hasShadow = false
            panel.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.mainMenuWindow)) + 3)
            panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
            panel.contentView = NSHostingView(rootView: PetChatView(state: state, onClose: { [weak self] in self?.closeChat() }))
            chatPanel = panel
        }
        chatPanel?.setFrame(frame, display: true)
        chatPanel?.makeKeyAndOrderFront(nil)          // it must be key so the text field can be typed in
        installKeyMonitor()
    }

    /// Escape closes the chat and the music panel (one monitor for both).
    private func installKeyMonitor() {
        guard keyMonitor == nil else { return }
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard event.keyCode == 53 else { return event }
            let consumed: Bool = MainActor.assumeIsolated {
                guard let self, let window = event.window else { return false }
                if window === self.chatPanel { self.closeChat(); return true }
                if window === self.mediaPanel { self.closeMedia(); return true }
                return false
            }
            return consumed ? nil : event
        }
    }

    private func removeKeyMonitorIfIdle() {
        guard chatPanel == nil, mediaPanel == nil, let keyMonitor else { return }
        NSEvent.removeMonitor(keyMonitor)
        self.keyMonitor = nil
    }

    func closeChat() {
        chatPanel?.orderOut(nil)
        chatPanel = nil
        removeKeyMonitorIfIdle()
    }

    private static func makePanel(frame: NSRect) -> NSPanel {
        let panel = NSPanel(contentRect: frame, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = false
        panel.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.mainMenuWindow)) + 3)
        panel.collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .ignoresCycle]
        return panel
    }
}

/// A borderless panel that can take the keyboard (for the chat's text field) without activating MIKA.
final class PetChatPanel: NSPanel {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect { frameRect }
}

// MARK: - Views

/// Música, the other agents and Settings around the circle, each a mini Mika (or an icon) in a dark round chip. The circle
/// sits at the top edge of the screen, so the ring is a half circle fanning out below it; `PetRing.layout` (twin of the
/// Windows `ringLayout`) puts the items on one arc with equal angle steps. With room the name is under the icon; where
/// there is none the items are bare 30 pt circles and the name shows only under the hovered one.
struct PetRingView: View {
    @ObservedObject var state: AppState
    let agents: [AgentDefinition]
    let frame: RingFrame
    let onPick: (String) -> Void
    let onCircle: () -> Void
    let onMusic: () -> Void
    let onSettings: () -> Void
    @State private var hoveredID: String?

    /// The "Música" item shows while a player has something playing or paused (the ring being open is what makes
    /// AppState ask the players, so it can take a moment to appear).
    private var showsMusic: Bool { state.mediaControl && state.nowPlaying?.isVisible == true }

    var body: some View {
        let spots = PetRing.layout(frame, agents: agents.count, music: showsMusic, narrower: PetRing.macNarrowing)
        let lead = spots.music == nil ? 0 : 1
        ZStack(alignment: .topLeading) {
            // The circle itself still takes the click (MIKA, or the agent in front).
            Circle()
                .fill(Color.white.opacity(0.001))
                .frame(width: 34, height: 34)
                .position(x: frame.cx, y: frame.cy)
                .onTapGesture(perform: onCircle)
            ForEach(Array(agents.enumerated()), id: \.element.id) { index, agent in
                if index < spots.agents.count {
                    PetRingButton(name: agent.name, help: agent.specialty, index: index + lead,
                                  compact: spots.compact, anchor: spots.agents[index],
                                  onHoverChange: { hoveredID = $0 ? agent.id : nil }, onTap: { onPick(agent.id) }) { size in
                        MiniBotCanvasView(task: state.tasks.first { $0.id == AgentTask.agentTaskID(agent.id) }
                                            ?? AgentTask.pill(for: agent, provider: agent.provider))
                            .frame(width: size / 0.6, height: size / 0.6)
                            .frame(width: size, height: size)
                    }
                    .zIndex(hoveredID == agent.id ? 1 : 0)
                }
            }
            if let music = spots.music {
                PetRingButton(name: "Música", help: "Música", index: 0, compact: spots.compact, anchor: music,
                              onHoverChange: { hoveredID = $0 ? "music" : nil }, onTap: onMusic) { size in
                    Image(systemName: "music.note")
                        .font(.system(size: size / 2, weight: .semibold))
                        .foregroundColor(Color(hex: "#F5F6F8"))
                        .frame(width: size, height: size)
                }
                .zIndex(hoveredID == "music" ? 1 : 0)
            }
            if let settings = spots.settings {
                PetRingButton(name: "Settings", help: "Ajustes", index: lead + agents.count, compact: spots.compact,
                              anchor: settings,
                              onHoverChange: { hoveredID = $0 ? "settings" : nil }, onTap: onSettings) { size in
                    Image(systemName: "gearshape")
                        .font(.system(size: size / 2, weight: .semibold))
                        .foregroundColor(Color(hex: "#F5F6F8"))
                        .frame(width: size, height: size)
                }
                .zIndex(hoveredID == "settings" ? 1 : 0)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .tooltipHost()
    }
}

/// One item of the ring. Labelled: the dark chip (34 pt) with its name under it, in the 64 x 52 box the layout reserves
/// (`PetRing.itemLeft/Top/Width/Height`; `anchor` is the chip's centre). Compact: the chip alone, 30 pt, centred on
/// `anchor`, and its name in a pill under it only while it is hovered.
private struct PetRingButton<Icon: View>: View {
    let name: String
    let help: String
    let index: Int
    let compact: Bool
    let anchor: CGPoint
    let onHoverChange: (Bool) -> Void
    let onTap: () -> Void
    /// The glyph for a given size (the chip adds 4 pt of padding on every side).
    let icon: (CGFloat) -> Icon
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var shown = false
    @State private var hovered = false

    init(name: String, help: String, index: Int, compact: Bool, anchor: CGPoint,
         onHoverChange: @escaping (Bool) -> Void, onTap: @escaping () -> Void,
         @ViewBuilder icon: @escaping (CGFloat) -> Icon) {
        self.name = name
        self.help = help
        self.index = index
        self.compact = compact
        self.anchor = anchor
        self.onHoverChange = onHoverChange
        self.onTap = onTap
        self.icon = icon
    }

    private static var chipFill: Color { Color(red: 10 / 255, green: 11 / 255, blue: 13 / 255) }

    private var chip: some View {
        icon(compact ? 22 : 26)
            .padding(4)
            .background(Circle().fill(Self.chipFill.opacity(0.92)))
            .overlay(Circle().stroke(Color.white.opacity(0.08), lineWidth: 1))
            .shadow(color: .black.opacity(0.45), radius: 6, x: 0, y: 4)
            .scaleEffect(hovered ? 1.12 : 1)
            .animation(.easeOut(duration: 0.15), value: hovered)
    }

    private var label: some View {
        Text(name)
            .font(.system(size: 9.5, weight: .bold))
            .foregroundColor(Color(hex: "#F5F6F8"))
            .lineLimit(1)
            .truncationMode(.tail)
            .padding(.horizontal, 5)
            .background(RoundedRectangle(cornerRadius: 6).fill(Self.chipFill.opacity(0.9)))
    }

    var body: some View {
        Button(action: onTap) {
            if compact {
                chip
                    .frame(width: PetRing.iconSize, height: PetRing.iconSize)
                    .overlay(alignment: .top) {
                        if hovered {
                            label.fixedSize().offset(y: PetRing.iconSize + 4)
                        }
                    }
                    .contentShape(Circle())
            } else {
                VStack(spacing: 2) {
                    chip
                    label
                }
                .padding(.top, 4)
                .frame(width: PetRing.itemWidth, height: PetRing.itemHeight, alignment: .top)
                .contentShape(Rectangle())
            }
        }
        .buttonStyle(.plain)
        // No tooltip here: the ring already names each item under its icon (or in a pill on hover).
        .accessibilityLabel(name)
        .accessibilityHint(help)
        .onHover { hovering in
            hovered = hovering
            onHoverChange(hovering)
        }
        // Labelled: the box's centre is 5 pt under the anchor (the chip's centre is 21 pt under the box's top).
        .position(x: anchor.x, y: compact ? anchor.y : anchor.y + (PetRing.itemHeight / 2 + PetRing.itemTop))
        .scaleEffect(shown ? 1 : 0.4)
        .opacity(shown ? 1 : 0)
        .onAppear {
            guard !reduceMotion else { shown = true; return }
            withAnimation(.spring(response: 0.32, dampingFraction: 0.62).delay(Double(index) * 0.04)) { shown = true }
        }
    }
}

/// The chat beside the circle: the island's chat (header, transcript, composer, copy, hand-offs) without the bot
/// column, on a black card with a close button.
struct PetChatView: View {
    @ObservedObject var state: AppState
    let onClose: () -> Void
    @State private var closeHovered = false

    var body: some View {
        ZStack(alignment: .topTrailing) {
            RoundedRectangle(cornerRadius: 22).fill(Color.black)
            RoundedRectangle(cornerRadius: 22).stroke(Color.white.opacity(0.08), lineWidth: 1)
            PromptView(state: state, leadingInset: 16)
                .padding(8)
            Button(action: onClose) {
                Image(systemName: "xmark")
                    .font(.system(size: 9, weight: .bold))
                    .foregroundColor(closeHovered ? Color(hex: "#F5F6F8") : Color(hex: "#8E939C"))
                    .frame(width: 20, height: 20)
                    .background(Circle().fill(Color.white.opacity(closeHovered ? 0.14 : 0.07)))
            }
            .buttonStyle(.plain)
            .onHover { closeHovered = $0 }
            .tip("Cerrar", iconOnly: true)
            .padding(.top, 16)
            .padding(.trailing, 18)
        }
        .clipShape(RoundedRectangle(cornerRadius: 22))
        .foregroundColor(Color(hex: "#F5F6F8"))
        .environment(\.colorScheme, .dark)
        .onExitCommand(perform: onClose)
        .tooltipHost()
    }
}

/// The music panel beside the circle: the media strip on a black card.
struct PetMediaView: View {
    @ObservedObject var state: AppState
    let onClose: () -> Void

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 16).fill(Color.black)
            RoundedRectangle(cornerRadius: 16).stroke(Color.white.opacity(0.08), lineWidth: 1)
            if let playing = state.nowPlaying, playing.isVisible {
                MediaStripView(playing: playing, onAction: { state.sendMedia($0) },
                               onVolume: { state.setMediaVolume($0) }, onMute: { state.toggleMediaMute() })
                    .frame(height: MediaStripMetrics.height)
                    .padding(.horizontal, 10)
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 16))
        .foregroundColor(Color(hex: "#F5F6F8"))
        .environment(\.colorScheme, .dark)
        .onExitCommand(perform: onClose)
        .tooltipHost()
    }
}
