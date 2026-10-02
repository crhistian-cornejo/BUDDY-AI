import AppKit
import SwiftUI

/// The composer and the chat panels next to Buddy, on the system's glass with the system's window shadow. Placed on
/// the side of the pet with more room, the chat growing upwards with its content (never past the screen).
@MainActor
final class ChatWindows {
    private let chat: ChatController
    private let pet: () -> NSRect
    private var composer: KeyPanel?
    private var panel: KeyPanel?
    private var contentHeight: CGFloat = 0
    private var clickMonitor: Any?
    private let history = HistoryWindow()
    private var moveObserver: NSObjectProtocol?

    static let gap: CGFloat = 8
    static let maxChatHeight: CGFloat = 520

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
            let view = ComposerView(chat: chat, onClose: { [weak self] in self?.close() })
            let host = NSHostingView(rootView: view)
            let size = CGSize(width: ChatMetrics.composerWidth, height: max(host.fittingSize.height, ChatMetrics.composerHeight))
            composer = KeyPanel.make(size: size, content: host, cornerRadius: size.height / 2)
            // The field grows with long text: the capsule follows.
            host.sizingOptions = [.intrinsicContentSize]
            NotificationCenter.default.addObserver(forName: NSView.frameDidChangeNotification, object: host, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.layout() }
            }
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

    /// A click outside Buddy's windows closes the chat, unless an answer is being written.
    private func closeIfClickedOutside() {
        let p = NSEvent.mouseLocation
        let inside = [composer?.frame, panel?.frame, pet(), history.frame].compactMap { $0 }.contains { $0.contains(p) }
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
            let host = NSHostingView(rootView: ChatView(
                chat: chat,
                onClose: { [weak self] in self?.close() },
                onHistory: { [weak self] in self?.showHistory() },
                onHeight: { [weak self] h in self?.contentHeight = h; self?.layout() }))
            // The window decides the size (it grows with the content up to a cap); SwiftUI fills it from the top.
            host.sizingOptions = []
            let p = KeyPanel.make(size: CGSize(width: ChatMetrics.chatWidth, height: 160), content: host,
                                  cornerRadius: ChatMetrics.cornerRadius)
            panel = p
            p.orderFront(nil)
        }
        layout()
    }

    private func layout() {
        guard let composer else { return }
        let pet = pet()
        let screen = NSScreen.screens.first { $0.frame.contains(CGPoint(x: pet.midX, y: pet.midY)) } ?? NSScreen.main
        let area = (screen?.visibleFrame ?? pet).insetBy(dx: Self.gap, dy: Self.gap)
        let height = max(composer.contentView?.fittingSize.height ?? 0, ChatMetrics.composerHeight)
        let size = CGSize(width: ChatMetrics.composerWidth, height: height)
        // The side of the pet with more room; bottom level with Buddy's feet.
        let left = pet.midX > area.midX
        var x = left ? pet.minX - size.width - Self.gap : pet.maxX + Self.gap
        x = min(max(x, area.minX), area.maxX - size.width)
        let y = min(max(pet.minY + 6, area.minY), area.maxY - size.height)
        composer.setFrame(NSRect(origin: CGPoint(x: x, y: y), size: size), display: true)
        composer.invalidateShadow()

        guard let panel else { return }
        let width = ChatMetrics.chatWidth
        let px = min(max(left ? x + size.width - width : x, area.minX), area.maxX - width)
        let bottom = y + size.height + Self.gap
        let chatHeight = min(max(contentHeight, 120), Self.maxChatHeight, area.maxY - bottom)
        panel.setFrame(NSRect(x: px, y: bottom, width: width, height: chatHeight), display: true)
        panel.invalidateShadow()
    }
}

/// A borderless panel on the system's glass (Liquid Glass on macOS 26 and later, the popover material before) with
/// rounded corners and the system window shadow. It can take the keyboard without a title bar.
final class KeyPanel: NSPanel {
    override var canBecomeKey: Bool { true }

    static func make(size: CGSize, content: NSView, cornerRadius: CGFloat) -> KeyPanel {
        let panel = KeyPanel(contentRect: NSRect(origin: .zero, size: size), styleMask: [.borderless, .nonactivatingPanel],
                             backing: .buffered, defer: false)
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        panel.isReleasedWhenClosed = false
        panel.hidesOnDeactivate = false
        panel.becomesKeyOnlyIfNeeded = false
        panel.contentView = GlassBackground.wrap(content, cornerRadius: cornerRadius)
        return panel
    }
}

enum GlassBackground {
    /// `content` on the system glass, clipped to the corner radius.
    @MainActor
    static func wrap(_ content: NSView, cornerRadius: CGFloat) -> NSView {
        if #available(macOS 26.0, *) {
            let glass = NSGlassEffectView()
            glass.cornerRadius = cornerRadius
            glass.contentView = content
            return glass
        }
        let effect = NSVisualEffectView()
        effect.material = .popover
        effect.blendingMode = .behindWindow
        effect.state = .active
        effect.wantsLayer = true
        effect.layer?.cornerRadius = cornerRadius
        effect.layer?.cornerCurve = .continuous
        effect.layer?.masksToBounds = true
        content.translatesAutoresizingMaskIntoConstraints = false
        effect.addSubview(content)
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: effect.leadingAnchor),
            content.trailingAnchor.constraint(equalTo: effect.trailingAnchor),
            content.topAnchor.constraint(equalTo: effect.topAnchor),
            content.bottomAnchor.constraint(equalTo: effect.bottomAnchor),
        ])
        return effect
    }
}
