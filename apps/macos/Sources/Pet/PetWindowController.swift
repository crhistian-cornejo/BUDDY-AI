import AppKit
import SwiftUI

/// The floating mascot window: borderless, transparent, above other windows, on every Space, never activating the
/// app. Dragging moves it; its place is saved per screen in the core's settings.
@MainActor
final class PetWindowController: NSObject, NSWindowDelegate {
    private let core: BuddyCore
    private let tokens: DesignTokens
    private let model: PetModel
    private let panel: NSPanel
    private var bubble: NSPanel?
    private var bubbleWork: DispatchWorkItem?
    private var saveWork: DispatchWorkItem?

    private var scale: Double { tokens.pet.scaleNormal }
    private var side: CGFloat { CGFloat(Double(model.size) * scale) }

    init(core: BuddyCore, sprite: Sprite, tokens: DesignTokens) {
        self.core = core
        self.tokens = tokens
        model = PetModel(sprite: sprite, motion: tokens.motion)
        let side = CGFloat(Double(sprite.size) * tokens.pet.scaleNormal)
        panel = Self.makePanel(size: CGSize(width: side, height: side))
        super.init()
        let host = PetHostingView(rootView: PetView(model: model, scale: scale))
        host.onQuit = { NSApp.terminate(nil) }
        panel.contentView = host
        panel.delegate = self
    }

    func show() {
        panel.setFrameOrigin(restoredOrigin())
        panel.orderFrontRegardless()
        model.start()
    }

    /// Shows `text` in a bubble next to Buddy for a few seconds. The bubble never takes clicks.
    func say(_ text: String) {
        bubbleWork?.cancel()
        bubble?.orderOut(nil)
        let host = NSHostingView(rootView: BubbleView(text: text, tokens: tokens))
        let panel = Self.makePanel(size: host.fittingSize)
        panel.ignoresMouseEvents = true
        panel.contentView = host
        bubble = panel
        placeBubble()
        panel.alphaValue = 0
        panel.orderFrontRegardless()
        NSAnimationContext.runAnimationGroup { $0.duration = 0.2; panel.animator().alphaValue = 1 }

        let work = DispatchWorkItem { [weak self] in
            guard let self, let bubble = self.bubble else { return }
            NSAnimationContext.runAnimationGroup({ $0.duration = self.tokens.motion.fadeOut
                bubble.animator().alphaValue = 0
            }, completionHandler: {
                MainActor.assumeIsolated {
                    bubble.orderOut(nil)
                    if self.bubble === bubble { self.bubble = nil }
                }
            })
        }
        bubbleWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + tokens.motion.helloSeconds, execute: work)
    }

    // MARK: NSWindowDelegate

    func windowDidMove(_ notification: Notification) {
        placeBubble()
        saveWork?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.saveOrigin() }
        saveWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
    }

    // MARK: Position

    private func placeBubble() {
        guard let bubble else { return }
        let pet = panel.frame
        let visible = (panel.screen ?? NSScreen.main)?.visibleFrame ?? pet
        let size = bubble.frame.size
        var origin = CGPoint(x: pet.midX - size.width / 2, y: pet.maxY - 4)
        if origin.y + size.height > visible.maxY { origin.y = pet.minY - size.height + 4 }
        origin.x = min(max(origin.x, visible.minX), visible.maxX - size.width)
        bubble.setFrameOrigin(origin)
    }

    private func saveOrigin() {
        guard let screen = panel.screen, let id = screen.displayID else { return }
        let origin = panel.frame.origin
        try? core.setSetting(key: "pet.screen", value: String(id))
        try? core.setSetting(key: "pet.origin.\(id)", value: "\(Int(origin.x)),\(Int(origin.y))")
    }

    /// The saved place on the saved screen, kept inside it; otherwise the bottom-right corner of the main screen.
    private func restoredOrigin() -> CGPoint {
        let margin = tokens.pet.margin
        if let savedID = (try? core.setting(key: "pet.screen")) ?? nil,
           let screen = NSScreen.screens.first(where: { $0.displayID.map(String.init) == savedID }),
           let saved = (try? core.setting(key: "pet.origin.\(savedID)")) ?? nil {
            let parts = saved.split(separator: ",").compactMap { Double($0) }
            if parts.count == 2 {
                let visible = screen.visibleFrame
                return CGPoint(x: min(max(parts[0], visible.minX), visible.maxX - side),
                               y: min(max(parts[1], visible.minY), visible.maxY - side))
            }
        }
        let visible = (NSScreen.main ?? NSScreen.screens[0]).visibleFrame
        return CGPoint(x: visible.maxX - side - margin, y: visible.minY + margin)
    }

    private static func makePanel(size: CGSize) -> NSPanel {
        let panel = NSPanel(contentRect: NSRect(origin: .zero, size: size),
                            styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .ignoresCycle]
        panel.isReleasedWhenClosed = false
        panel.hidesOnDeactivate = false
        return panel
    }
}

/// Drags the window natively on mouse down; right click shows the menu.
private final class PetHostingView: NSHostingView<PetView> {
    var onQuit: (() -> Void)?

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func mouseDown(with event: NSEvent) {
        window?.performDrag(with: event)
    }

    override func menu(for event: NSEvent) -> NSMenu? {
        let menu = NSMenu()
        let quit = NSMenuItem(title: "Salir de Buddy", action: #selector(quit), keyEquivalent: "")
        quit.target = self
        menu.addItem(quit)
        return menu
    }

    @objc private func quit() { onQuit?() }
}

private extension NSScreen {
    var displayID: UInt32? {
        (deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value
    }
}
