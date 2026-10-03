import AppKit
import SwiftUI

/// The floating mascot window: borderless, transparent, above other windows, on every Space, never activating the
/// app. The core's PetBrain decides what Buddy does while idle (including sitting down bored after a while without
/// being used); this class plays it, walks the window inside the screen, handles hover, drag and click, and saves the
/// place per screen.
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
    /// Last time Buddy was used (hover, click, drag, chat, work, talk, walk): PetBrain sits it down 10 s later.
    private var lastUse = Date()
    /// Holding the seated frame (the last plan rested on `sit`).
    private var seated = false
    private var sleeping: Bool { ["lie-down", "sleep", "sleep-still"].contains(model.state) }
    private var hovering = false
    /// The life loop is waiting before its next plan (so a use can re-plan at once).
    private var waiting = false

    /// A click on Buddy (not a drag).
    var onClick: (() -> Void)?
    /// «Historial de chats» from the right-click menu.
    var onHistory: (() -> Void)?
    /// While the chat is open Buddy stays put (no walks): the chat hangs from it. It also keeps Buddy standing.
    var holdStill = false {
        didSet { if holdStill != oldValue { used() } }
    }

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
        host.onClick = { [weak self] in
            self?.used()
            self?.onClick?()
        }
        host.onHover = { [weak self] inside in
            guard let self else { return }
            self.hovering = inside
            if inside { self.used() } else { self.lastUse = Date() }
        }
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

    /// Asks the core's PetBrain for the next plan, waits, plays it, and again. `standUp` first gets a seated Buddy up.
    private func startLife(standUp: Bool = false, wakeUp: Bool = false) {
        life?.cancel()
        waiting = false
        life = Task { [weak self] in
            if standUp, let self {
                if NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
                    self.model.show("idle")
                } else {
                    await self.model.play(wakeUp ? "wake-up" : "stand-up", duration: 0)
                }
            }
            while !Task.isCancelled {
                guard let self else { return }
                let plan = self.brain.next(ctx: self.context())
                #if DEBUG
                NSLog("Buddy pet plan: %@%@ wait %d ms, %d ms, dx %.0f, rest %@", plan.intro.isEmpty ? "" : plan.intro + " → ",
                      plan.state, plan.waitMs, plan.durationMs, plan.dx, plan.rest)
                #endif
                self.waiting = true
                try? await Task.sleep(for: .milliseconds(Int(plan.waitMs)))
                if Task.isCancelled { return }
                self.waiting = false
                if self.activity != nil { continue }
                if plan.dx != 0 {
                    // The chat or the menu may have disabled walks while this plan was waiting.
                    if self.holdStill || !self.wander || self.context().reduceMotion { continue }
                    if !plan.intro.isEmpty, !(await self.model.play(plan.intro, duration: 0)) { continue }
                    self.seated = false
                    await self.walk(plan)
                    if Task.isCancelled { return }
                    self.lastUse = Date()
                } else {
                    if !plan.intro.isEmpty, !(await self.model.play(plan.intro, duration: 0, rest: plan.rest)) { continue }
                    let done = await self.model.play(plan.state, duration: Double(plan.durationMs) / 1000, rest: plan.rest)
                    if done && !Task.isCancelled { self.seated = plan.rest == "sit" }
                }
            }
        }
    }

    /// Buddy was used: the count to sitting restarts, a seated Buddy stands up and a waiting plan is made again.
    private func used(standUp: Bool = true) {
        lastUse = Date()
        if seated || sleeping {
            let wakeUp = sleeping
            seated = false
            startLife(standUp: standUp && activity == nil, wakeUp: wakeUp)
        } else if waiting {
            startLife()
        }
    }

    private func context() -> PetContext {
        let visible = (panel.screen ?? NSScreen.main)?.visibleFrame ?? panel.frame
        let idle = CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: CGEventType(rawValue: ~0)!)
        return PetContext(x: panel.frame.minX, minX: visible.minX, maxX: visible.maxX - panel.frame.width,
                          reduceMotion: NSWorkspace.shared.accessibilityDisplayShouldReduceMotion,
                          wander: wander && !holdStill, idleSeconds: idle,
                          untouchedSeconds: activity != nil ? 0 : Date().timeIntervalSince(lastUse),
                          engaged: hovering || holdStill, sitting: seated, sleeping: sleeping)
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

    /// Buddy is sitting at its laptop (glasses on): thinking and working show this instead of the plain states.
    private var atLaptop = false
    private static let laptopStates = ["think": "laptop-think", "work": "laptop-type"]

    /// Closes the laptop, takes the glasses off and stands up, if Buddy was at it.
    private func leaveLaptop() async {
        guard atLaptop else { return }
        atLaptop = false
        if model.has("laptop-off") { await model.play("laptop-off", duration: 0) }
    }

    /// Shows an agent state until `nil` (phase 1: thinking, working, asking, error, done). Thinking and working
    /// put the glasses on, sit down and open a laptop; any other state (or none) puts it all away first.
    func setActivity(_ state: String?, leaveLaptop leave: Bool = true) {
        activityTask?.cancel()
        let wasBusy = activity != nil
        let wasSeated = seated
        let wasSleeping = sleeping
        activity = state
        if state != nil || wasBusy { used(standUp: false) }
        let laptop = state.flatMap { Self.laptopStates[$0] }.flatMap { model.has($0) ? $0 : nil }
        guard let state, laptop != nil || model.has(state) else {
            if atLaptop && leave {
                activityTask = Task { [weak self] in
                    await self?.leaveLaptop()
                    if let self, !Task.isCancelled, !self.seated, !self.sleeping { self.model.show("idle") }
                }
            } else if !seated && !sleeping {
                model.show("idle")
            }
            return
        }
        activityTask = Task { [weak self] in
            guard let self else { return }
            let reduce = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
            if let laptop {
                if !self.atLaptop {
                    if (wasSeated || wasSleeping) && !reduce {
                        guard await self.model.play(wasSleeping ? "wake-up" : "stand-up", duration: 0) else { return }
                    }
                    self.atLaptop = true
                    if !reduce {
                        guard await self.model.play("laptop-on", duration: 0, rest: "laptop-type") else { return }
                    }
                }
                await self.model.loop(laptop)
                return
            }
            await self.leaveLaptop()
            if Task.isCancelled { return }
            if (wasSeated || wasSleeping) && !reduce {
                guard await self.model.play(wasSleeping ? "wake-up" : "stand-up", duration: 0) else { return }
            }
            await self.model.loop(state)
        }
    }

    /// A mascot state from the core: agent states loop until the next one; done and error play once.
    func showMascotState(_ state: String) {
        switch state {
        case "think", "work", "ask", "listen":
            setActivity(state)
        case "done", "error":
            setActivity(nil, leaveLaptop: false)   // the reaction puts the laptop away first
            react(state)
        default:
            setActivity(nil)
        }
    }

    /// Plays a one-off reaction (done, wave…) and returns to whatever was showing.
    func react(_ state: String) {
        used(standUp: false)
        Task { [weak self] in
            guard let self else { return }
            await self.leaveLaptop()
            await self.model.play(state, duration: 1)
            if let activity = self.activity { self.setActivity(activity) }
        }
    }

    // MARK: Drag

    private func dragStarted() {
        life?.cancel()
        seated = false
        lastUse = Date()
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
        lastUse = Date()
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
        let history = NSMenuItem(title: "Historial de chats", action: #selector(openHistory), keyEquivalent: "f")
        history.target = self
        history.image = NSImage(systemSymbolName: "clock.arrow.circlepath", accessibilityDescription: nil)
        menu.addItem(history)
        if let news = newsMenu() { menu.addItem(news) }
        menu.addItem(.separator())
        let walk = NSMenuItem(title: "Pasear por la pantalla", action: #selector(toggleWander), keyEquivalent: "p")
        walk.keyEquivalentModifierMask = [.command, .shift]
        walk.image = NSImage(systemSymbolName: "figure.walk", accessibilityDescription: nil)
        walk.target = self
        walk.state = wander ? .on : .off
        menu.addItem(walk)
        let settings = NSMenuItem(title: "Ajustes…", action: #selector(openSettings), keyEquivalent: ",")
        settings.target = self
        settings.image = NSImage(systemSymbolName: "gearshape", accessibilityDescription: nil)
        menu.addItem(settings)
        menu.addItem(.separator())
        let quit = NSMenuItem(title: "Salir de Buddy", action: #selector(quit), keyEquivalent: "q")
        quit.target = self
        quit.image = NSImage(systemSymbolName: "power", accessibilityDescription: nil)
        menu.addItem(quit)
        return menu
    }

    /// Today's «mensajitos», compact: a header per topic, one short line each (the whole sentence in its tooltip).
    private func newsMenu() -> NSMenuItem? {
        let latest = Array(core.briefing().prefix(5))
        guard !latest.isEmpty else { return nil }
        // Grouped by topic, in the order the topics first appear.
        let topics = latest.map(\.topic).reduce(into: [String]()) { if !$0.contains($1) { $0.append($1) } }
        let news = topics.flatMap { t in latest.filter { $0.topic == t } }
        let item = NSMenuItem(title: "Novedades de hoy", action: nil, keyEquivalent: "")
        item.image = NSImage(systemSymbolName: "newspaper", accessibilityDescription: nil)
        let sub = NSMenu()
        var topic: String?
        for line in news {
            let name = line.topic.isEmpty ? "Novedades" : line.topic.prefix(1).uppercased() + line.topic.dropFirst()
            if name != topic {
                sub.addItem(.sectionHeader(title: name))
                topic = name
            }
            let entry = NSMenuItem(title: Self.short(line.text, 46), action: line.url == nil ? nil : #selector(openNews(_:)), keyEquivalent: "")
            entry.target = self
            entry.representedObject = line.url
            entry.toolTip = line.text
            sub.addItem(entry)
        }
        item.submenu = sub
        return item
    }

    private static func short(_ text: String, _ limit: Int) -> String {
        text.count <= limit ? text : String(text.prefix(limit - 1)).trimmingCharacters(in: .whitespaces) + "…"
    }

    @objc private func openHistory() { onHistory?() }

    @objc func toggleWander() {
        try? core.setSetting(key: "pet.wander", value: wander ? "false" : "true")
    }

    @objc private func quit() { NSApp.terminate(nil) }

    /// A mensajito's source: web links only.
    @objc private func openNews(_ sender: NSMenuItem) {
        guard let link = sender.representedObject as? String, let url = URL(string: link),
              ["https", "http"].contains(url.scheme?.lowercased() ?? "") else { return }
        NSWorkspace.shared.open(url)
    }

    /// The system's Settings window (an LSUIElement app has no menu for it).
    @objc private func openSettings() {
        SettingsWindow.show()
    }

    // MARK: Bubble

    /// Shows `text` in a bubble next to Buddy for a few seconds. The bubble never takes clicks.
    func say(_ text: String, seconds: TimeInterval? = nil) {
        used()
        bubbleWork?.cancel()
        bubble?.orderOut(nil)
        let host = NSHostingView(rootView: BubbleView(text: text).buddySurface(cornerRadius: 18))
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
        // The bubble's window has the surface margin around the shape.
        let m = Surface.margin
        let size = CGSize(width: bubble.frame.width - 2 * m, height: bubble.frame.height - 2 * m)
        var origin = CGPoint(x: pet.midX - size.width / 2, y: pet.maxY + 4)
        if origin.y + size.height > visible.maxY { origin.y = pet.minY - size.height - 4 }
        origin.x = min(max(origin.x, visible.minX), visible.maxX - size.width)
        bubble.setFrameOrigin(CGPoint(x: origin.x - m, y: origin.y - m))
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
    var onHover: ((Bool) -> Void)?
    var menuProvider: (() -> NSMenu?)?

    private var downAt: CGPoint?
    private var hoverArea: NSTrackingArea?

    /// Enter and exit only (no mouse-moved events): hovering stands Buddy up and keeps it standing.
    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let hoverArea { removeTrackingArea(hoverArea) }
        let area = NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect],
                                  owner: self, userInfo: nil)
        addTrackingArea(area)
        hoverArea = area
    }

    override func mouseEntered(with event: NSEvent) {
        super.mouseEntered(with: event)
        onHover?(true)
    }

    override func mouseExited(with event: NSEvent) {
        super.mouseExited(with: event)
        onHover?(false)
    }
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
