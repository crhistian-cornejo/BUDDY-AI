import AppKit
import SwiftUI

/// The composer and the chat panels next to Buddy: placed on the side of the pet with more room, the chat growing
/// upwards with its content (never past the screen), following the pet when it walks or is dragged.
@MainActor
final class ChatWindows {
    private let chat: ChatController
    private let tokens: DesignTokens
    private let pet: () -> NSRect
    private var composer: KeyPanel?
    private var panel: KeyPanel?
    private var contentHeight: CGFloat = 0
    private var clickMonitor: Any?
    private var moveObserver: NSObjectProtocol?

    static let composerSize = CGSize(width: 380, height: 52)
    static let chatWidth: CGFloat = 430
    static let maxChatHeight: CGFloat = 520

    var isOpen: Bool { composer != nil }
    /// Told when the chat opens or closes (Buddy holds still meanwhile).
    var onOpenChange: ((Bool) -> Void)?

    init(chat: ChatController, tokens: DesignTokens, pet: @escaping () -> NSRect) {
        self.chat = chat
        self.tokens = tokens
        self.pet = pet
    }

    func toggle() {
        isOpen ? close() : open()
    }

    func open() {
        if composer == nil {
            let panel = KeyPanel.make(size: Self.composerSize)
            panel.contentView = NSHostingView(rootView: ComposerView(chat: chat, tokens: tokens, onClose: { [weak self] in self?.close() })
                .padding(4))
            composer = panel
        }
        NSApp.activate()
        composer?.makeKeyAndOrderFront(nil)
        onOpenChange?(true)
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

    func close() {
        if composer != nil { onOpenChange?(false) }
        composer?.orderOut(nil)
        panel?.orderOut(nil)
        composer = nil
        panel = nil
        if let clickMonitor { NSEvent.removeMonitor(clickMonitor) }
        clickMonitor = nil
        if let moveObserver { NotificationCenter.default.removeObserver(moveObserver) }
        moveObserver = nil
    }

    /// A click elsewhere (outside Buddy's windows) closes the chat, unless an answer is being written.
    private func closeIfClickedOutside() {
        let p = NSEvent.mouseLocation
        let inside = [composer?.frame, panel?.frame, pet()].compactMap { $0 }.contains { $0.contains(p) }
        if !inside && !chat.streaming { close() }
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
            let p = KeyPanel.make(size: CGSize(width: Self.chatWidth, height: 160))
            p.contentView = NSHostingView(rootView: ChatView(
                chat: chat, tokens: tokens,
                onClose: { [weak self] in self?.close() },
                onHeight: { [weak self] h in self?.contentHeight = h; self?.layout() }))
            panel = p
            p.orderFront(nil)
        }
        layout()
    }

    private func layout() {
        guard let composer else { return }
        let pet = pet()
        let screen = NSScreen.screens.first { $0.frame.contains(CGPoint(x: pet.midX, y: pet.midY)) } ?? NSScreen.main
        let area = screen?.visibleFrame ?? pet
        let size = composer.frame.size
        // The side of the pet with more room.
        let left = pet.midX > area.midX
        var x = left ? pet.minX - size.width - 6 : pet.maxX + 6
        x = min(max(x, area.minX + 8), area.maxX - size.width - 8)
        var y = pet.minY + 8
        y = min(max(y, area.minY + 8), area.maxY - size.height - 8)
        composer.setFrameOrigin(CGPoint(x: x, y: y))

        guard let panel else { return }
        let width = Self.chatWidth
        let px = left ? x + size.width - width : x
        let room = area.maxY - (y + size.height + 6) - 8
        let height = min(max(contentHeight, 120), Self.maxChatHeight, room)
        panel.setFrame(NSRect(x: min(max(px, area.minX + 8), area.maxX - width - 8), y: y + size.height + 6,
                              width: width, height: height), display: true)
    }
}

/// A borderless panel that can take the keyboard (to type) without a title bar.
final class KeyPanel: NSPanel {
    override var canBecomeKey: Bool { true }

    static func make(size: CGSize) -> KeyPanel {
        let panel = KeyPanel(contentRect: NSRect(origin: .zero, size: size), styleMask: [.borderless, .nonactivatingPanel],
                             backing: .buffered, defer: false)
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        panel.isReleasedWhenClosed = false
        panel.hidesOnDeactivate = false
        panel.becomesKeyOnlyIfNeeded = false
        return panel
    }
}
