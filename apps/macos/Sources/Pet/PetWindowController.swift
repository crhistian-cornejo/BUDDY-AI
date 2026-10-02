import AppKit
import SwiftUI

/// The floating mascot window: borderless, transparent, above other windows, on every Space, never activating the
/// app. The core's PetBrain decides what Buddy does while idle; this class plays it, walks the window inside the
/// screen, handles drag and click, and saves the place per screen.
@MainActor
final class PetWindowController: NSObject, NSWindowDelegate {
    private let core: BuddyCore
    private let tokens: DesignTokens
    private let brain = PetBrain()
    let model: PetModel
    private let panel: NSPanel
    private var bubble: NSPanel?
    private var bubbleWork: DispatchWorkItem?
    private var saveWork: DispatchWorkItem?
    private var life: Task<Void, Never>?
    private var activityTask: Task<Void, Never>?
    /// An agent state shown until cleared (think, work, ask, error…); idle behavior pauses meanwhile.
    private(set) var activity: String?

    /// A click on Buddy (not a drag).
    var onClick: (() -> Void)?
    /// While the chat is open Buddy stays put (no walks): the chat hangs from it.
    var holdStill = false

    private var scale: Double { tokens.pet.scaleNormal }
    private var side: CGFloat { CGFloat(Double(model.size) * scale) }
    var frame: NSRect { panel.frame }

    init(core: BuddyCore, sprite: Sprite, tokens: DesignTokens) {
        self.core = core
        self.tokens = tokens
        model = PetModel(sprite: sprite, maxFps: tokens.motion.maxFps)
        let side = CGFloat(Double(sprite.size) * tokens.pet.scaleNormal)
        panel = Self.makePanel(size: CGSize(width: side, height: side))
        super.init()
        let host = PetHostingView(rootView: PetView(model: model, scale: scale))
        host.onDragStart = { [weak self] in self?.dragStarted() }
        host.onDragEnd = { [weak self] in self?.dragEnded() }
        host.onClick = { [weak self] in self?.onClick?() }
        host.menuProvider = { [weak self] in self?.menu() }
        panel.contentView = host
        panel.delegate = self
    }

    func show() {
        panel.setFrameOrigin(restoredOrigin())
        panel.orderFrontRegardless()
        startLife()
        Task { await model.play("wave", duration: 1) }
    }

    // MARK: Life

    private var wander: Bool {
        ((try? core.setting(key: "pet.wander")) ?? nil) != "false"
    }

    private func startLife() {
        life?.cancel()
        life = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                let plan = self.brain.next(ctx: self.context())
                #if DEBUG
                NSLog("Buddy pet plan: %@ wait %d ms, %d ms, dx %.0f", plan.state, plan.waitMs, plan.durationMs, plan.dx)
                #endif
                try? await Task.sleep(for: .milliseconds(Int(plan.waitMs)))
                if Task.isCancelled { return }
                if self.activity != nil { continue }
                if plan.dx != 0 {
                    await self.walk(plan)
                } else {
                    await self.model.play(plan.state, duration: Double(plan.durationMs) / 1000)
                }
            }
        }
    }

    private func context() -> PetContext {
        let visible = (panel.screen ?? NSScreen.main)?.visibleFrame ?? panel.frame
        let idle = CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: CGEventType(rawValue: ~0)!)
        return PetContext(x: panel.frame.minX, minX: visible.minX, maxX: visible.maxX - panel.frame.width,
                          reduceMotion: NSWorkspace.shared.accessibilityDisplayShouldReduceMotion,
                          wander: wander && !holdStill, idleSeconds: idle)
    }

    /// Steps the window sideways once per frame (pixel-art stepping), never past the screen's edges.
    private func walk(_ plan: PetPlan) async {
        let duration = Double(plan.durationMs) / 1000
        guard duration > 0 else { return }
        let speed = plan.dx / duration
        await model.play(plan.state, duration: duration) { [weak self] interval in
            guard let self else { return }
            var frame = self.panel.frame
            frame.origin.x += speed * interval
            self.panel.setFrameOrigin(self.clamped(frame).origin)
        }
    }

    /// Shows an agent state until `nil` (phase 1: thinking, working, asking, error, done).
    func setActivity(_ state: String?) {
        activityTask?.cancel()
        activity = state
        guard let state, model.has(state) else { model.show("idle"); return }
        activityTask = Task { [weak self] in await self?.model.loop(state) }
    }

    /// A mascot state from the core: agent states loop until the next one; done and error play once.
    func showMascotState(_ state: String) {
        switch state {
        case "think", "work", "ask", "listen":
            setActivity(state)
        case "done", "error":
            setActivity(nil)
            react(state)
        default:
            setActivity(nil)
        }
    }

    /// Plays a one-off reaction (done, wave…) and returns to whatever was showing.
    func react(_ state: String) {
        Task { [weak self] in
            guard let self else { return }
            await self.model.play(state, duration: 1)
            if let activity = self.activity { self.setActivity(activity) }
        }
    }

    // MARK: Drag

    private func dragStarted() {
        life?.cancel()
        activityTask?.cancel()
        activityTask = Task { [weak self] in await self?.model.loop("drag") }
    }

    private func dragEnded() {
        activityTask?.cancel()
        let target = clamped(panel.frame)
        if target != panel.frame {
            NSAnimationContext.runAnimationGroup { ctx in
                ctx.duration = tokens.motion.springResponse * 0.6
                panel.animator().setFrame(target, display: true)
            }
        }
        model.show("idle")
        if let activity { setActivity(activity) }
        saveOrigin()
        startLife()
    }

    /// Inside the screen's usable area (below the menu bar, above the Dock), corners included. Rule in the core.
    private func clamped(_ frame: NSRect) -> NSRect {
        let screen = NSScreen.screens.first { $0.frame.contains(CGPoint(x: frame.midX, y: frame.midY)) }
            ?? panel.screen ?? NSScreen.main
        guard let area = screen?.visibleFrame else { return frame }
        let r = clampToArea(window: PetRect(x: frame.minX, y: frame.minY, width: frame.width, height: frame.height),
                            area: PetRect(x: area.minX, y: area.minY, width: area.width, height: area.height))
        return NSRect(x: r.x, y: r.y, width: r.width, height: r.height)
    }

    // MARK: Menu

    private func menu() -> NSMenu {
        let menu = NSMenu()
        let walk = NSMenuItem(title: "Pasear por la pantalla", action: #selector(toggleWander), keyEquivalent: "")
        walk.target = self
        walk.state = wander ? .on : .off
        menu.addItem(walk)
        menu.addItem(.separator())
        let quit = NSMenuItem(title: "Salir de Buddy", action: #selector(quit), keyEquivalent: "")
        quit.target = self
        menu.addItem(quit)
        return menu
    }

    @objc private func toggleWander() {
        try? core.setSetting(key: "pet.wander", value: wander ? "false" : "true")
    }

    @objc private func quit() { NSApp.terminate(nil) }

    // MARK: Bubble

    /// Shows `text` in a bubble next to Buddy for a few seconds. The bubble never takes clicks.
    func say(_ text: String, seconds: TimeInterval? = nil) {
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
        DispatchQueue.main.asyncAfter(deadline: .now() + (seconds ?? tokens.motion.helloSeconds), execute: work)
    }

    // MARK: NSWindowDelegate

    func windowDidMove(_ notification: Notification) {
        placeBubble()
        NotificationCenter.default.post(name: .petMoved, object: nil)
        saveWork?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.saveOrigin() }
        saveWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5, execute: work)
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
           NSScreen.screens.contains(where: { $0.displayID.map(String.init) == savedID }),
           let saved = (try? core.setting(key: "pet.origin.\(savedID)")) ?? nil {
            let parts = saved.split(separator: ",").compactMap { Double($0) }
            if parts.count == 2 {
                return clamped(NSRect(x: parts[0], y: parts[1], width: side, height: side)).origin
            }
        }
        let visible = (NSScreen.main ?? NSScreen.screens[0]).visibleFrame
        return CGPoint(x: visible.maxX - side - margin, y: visible.minY + margin)
    }

    static func makePanel(size: CGSize) -> NSPanel {
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

extension Notification.Name {
    static let petMoved = Notification.Name("buddy.petMoved")
}

/// Tells a click from a drag: past 3 pt of movement it is a drag and the window follows the pointer.
private final class PetHostingView: NSHostingView<PetView> {
    var onDragStart: (() -> Void)?
    var onDragEnd: (() -> Void)?
    var onClick: (() -> Void)?
    var menuProvider: (() -> NSMenu?)?

    private var downAt: CGPoint?
    private var startOrigin: CGPoint = .zero
    private var dragging = false

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func mouseDown(with event: NSEvent) {
        downAt = NSEvent.mouseLocation
        startOrigin = window?.frame.origin ?? .zero
        dragging = false
    }

    override func mouseDragged(with event: NSEvent) {
        guard let downAt, let window else { return }
        let now = NSEvent.mouseLocation
        if !dragging, hypot(now.x - downAt.x, now.y - downAt.y) > 3 {
            dragging = true
            onDragStart?()
        }
        if dragging {
            window.setFrameOrigin(CGPoint(x: startOrigin.x + now.x - downAt.x, y: startOrigin.y + now.y - downAt.y))
        }
    }

    override func mouseUp(with event: NSEvent) {
        defer { downAt = nil; dragging = false }
        if dragging { onDragEnd?() } else if downAt != nil { onClick?() }
    }

    override func menu(for event: NSEvent) -> NSMenu? { menuProvider?() }
}

extension NSScreen {
    var displayID: UInt32? {
        (deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value
    }
}
