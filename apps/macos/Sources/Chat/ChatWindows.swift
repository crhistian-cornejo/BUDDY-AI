import AppKit
import SwiftUI

/// The composer and the chat next to Buddy. Placed on the side of the pet with more room, the chat growing upwards
/// with its content (never past the screen). Any click outside closes them and gives the keyboard back to the app
/// that had it.
@MainActor
final class ChatWindows {
    private let chat: ChatController
    private let pet: () -> NSRect
    private var composer: KeyPanel?
    private var composerHost: NSHostingView<AnyView>?
    private var panel: KeyPanel?
    private var contentHeight: CGFloat = 0
    /// The composer's own height (it grows with the text, up to its line limit).
    private var composerContentHeight: CGFloat = 0
    private var clickMonitor: Any?
    private var moveObserver: NSObjectProtocol?
    private let history = HistoryWindow()
    /// The app that had the keyboard before the chat opened.
    private var previousApp: NSRunningApplication?

    static let gap: CGFloat = 8
    static let maxChatHeight: CGFloat = 400

    var isOpen: Bool { composer != nil }
    /// Told when the chat opens or closes (Buddy holds still meanwhile).
    var onOpenChange: ((Bool) -> Void)?

    init(chat: ChatController, pet: @escaping () -> NSRect) {
        self.chat = chat
        self.pet = pet
    }

    func toggle() {
        isOpen ? close() : open()
    }

    func open() {
        if composer == nil {
            // Measured at its ideal height whatever the window is, so the window can follow the text as it wraps.
            let view = ComposerView(chat: chat, onClose: { [weak self] in self?.close() })
                .fixedSize(horizontal: false, vertical: true)
                .onGeometryChange(for: CGFloat.self, of: { $0.size.height }) { [weak self] height in
                    self?.composerHeightChanged(height)
                }
                .frame(maxHeight: .infinity, alignment: .top)
            let (panel, host) = KeyPanel.make(visibleSize: CGSize(width: ChatMetrics.composerWidth, height: ChatMetrics.composerHeight),
                                              cornerRadius: ChatMetrics.composerHeight / 2, prominent: true, view: view)
            composer = panel
            composerHost = host
            trackDraft()
        }
        let front = NSWorkspace.shared.frontmostApplication
        if front?.bundleIdentifier != Bundle.main.bundleIdentifier { previousApp = front }
        NSApp.activate()
        composer?.makeKeyAndOrderFront(nil)
        onOpenChange?(true)
        chat.prewarm()
        observeChat()
        updateChatPanel()
        layout()
        clickMonitor = clickMonitor ?? NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] _ in
            MainActor.assumeIsolated { self?.closeIfClickedOutside() }
        }
        moveObserver = moveObserver ?? NotificationCenter.default.addObserver(forName: .petMoved, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.layout() }
        }
    }

    /// Long text makes the field taller: the capsule follows.
    private func trackDraft() {
        guard composer != nil else { return }
        withObservationTracking { _ = chat.draft; _ = chat.attachments } onChange: { [weak self] in
            Task { @MainActor in self?.layout(); self?.trackDraft() }
        }
    }

    /// The history search in the middle of the screen; picking a chat opens it here.
    func showHistory() {
        let pet = pet()
        let screen = NSScreen.screens.first { $0.frame.contains(CGPoint(x: pet.midX, y: pet.midY)) }
        history.show(chat: chat, on: screen) { [weak self] id in
            guard let self else { return }
            self.chat.open(id)
            self.open()
        }
    }

    func close() {
        guard composer != nil else { return }
        onOpenChange?(false)
        composer?.orderOut(nil)
        panel?.orderOut(nil)
        history.close()
        composer = nil
        composerHost = nil
        composerContentHeight = 0
        panel = nil
        contentHeight = 0
        if let clickMonitor { NSEvent.removeMonitor(clickMonitor) }
        clickMonitor = nil
        if let moveObserver { NotificationCenter.default.removeObserver(moveObserver) }
        moveObserver = nil
        // The keyboard goes back where it was (an answer still being written keeps going and is in the history).
        previousApp?.activate()
        previousApp = nil
    }

    /// A click anywhere outside Buddy's windows closes the chat.
    private func closeIfClickedOutside() {
        let p = NSEvent.mouseLocation
        var mine = [composer, panel].compactMap { $0 }.map(Surface.visibleFrame(of:))
        mine.append(pet())
        if let frame = history.frame { mine.append(frame) }
        if !mine.contains(where: { $0.contains(p) }) { close() }
    }

    /// Shows the chat panel once there are messages, and keeps watching.
    private func observeChat() {
        withObservationTracking { _ = chat.messages.count } onChange: { [weak self] in
            Task { @MainActor in
                guard let self, self.isOpen else { return }
                self.updateChatPanel()
                self.observeChat()
            }
        }
    }

    private func updateChatPanel() {
        guard chat.hasContent else {
            panel?.orderOut(nil)
            panel = nil
            return
        }
        if panel == nil {
            let view = ChatView(
                chat: chat,
                onClose: { [weak self] in self?.close() },
                onHistory: { [weak self] in self?.showHistory() },
                onHeight: { [weak self] h in self?.contentHeightChanged(h) })
            let (p, _) = KeyPanel.make(visibleSize: CGSize(width: ChatMetrics.chatWidth, height: 160),
                                       cornerRadius: ChatMetrics.cornerRadius, view: view)
            panel = p
            p.orderFront(nil)
        }
        layout()
    }

    private func composerHeightChanged(_ height: CGFloat) {
        let rounded = height.rounded(.up)
        guard abs(rounded - composerContentHeight) >= 1 else { return }
        composerContentHeight = rounded
        layout()
    }

    /// Only real changes resize the window (a resize must never feed back into the measured height).
    private func contentHeightChanged(_ height: CGFloat) {
        let rounded = height.rounded(.up)
        guard abs(rounded - contentHeight) >= 1 else { return }
        contentHeight = rounded
        layout()
    }

    private func layout() {
        guard let composer else { return }
        let pet = pet()
        let screen = NSScreen.screens.first { $0.frame.contains(CGPoint(x: pet.midX, y: pet.midY)) } ?? NSScreen.main
        let area = (screen?.visibleFrame ?? pet).insetBy(dx: Self.gap, dy: Self.gap)
        let size = CGSize(width: ChatMetrics.composerWidth, height: max(composerContentHeight, ChatMetrics.composerHeight))
        // The side of the pet with more room; bottom level with Buddy's feet.
        let left = pet.midX > area.midX
        var x = left ? pet.minX - size.width - Self.gap : pet.maxX + Self.gap
        x = min(max(x, area.minX), area.maxX - size.width)
        let y = min(max(pet.minY + 6, area.minY), area.maxY - size.height)
        let composerFrame = Surface.windowFrame(for: NSRect(origin: CGPoint(x: x, y: y), size: size))
        if composer.frame != composerFrame { composer.setFrame(composerFrame, display: true) }

        guard let panel else { return }
        let width = ChatMetrics.chatWidth
        let px = min(max(left ? x + size.width - width : x, area.minX), area.maxX - width)
        let bottom = y + size.height + Self.gap
        let height = min(max(contentHeight, 120), Self.maxChatHeight, area.maxY - bottom)
        let chatFrame = Surface.windowFrame(for: NSRect(x: px, y: bottom, width: width, height: height))
        if panel.frame != chatFrame { panel.setFrame(chatFrame, display: true) }
    }
}
