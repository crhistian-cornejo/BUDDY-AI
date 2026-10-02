import AppKit
import Combine
import SwiftUI

@MainActor
final class IslandWindowController: NSWindowController {

    private var islandPanel: IslandPanel!
    private var state: AppState { AppState.shared }

    // State machine (replaces all hover/absence/auto-close timers)
    let fsm = IslandStateMachine()

    private var wasInIsland = false
    /// The main agent's tip, every few seconds, under the notch.
    private lazy var tips = TipController(state: AppState.shared)
    /// The circle that splits off beside the notch while the island is hidden.
    private lazy var satellite = SatelliteController(state: AppState.shared)
    private lazy var usageBubble = UsageBubbleController(state: AppState.shared)
    // The pet (a ring of agents on hover and a chat panel under the circle) is off on the Mac for now: the circle only gives
    // data, and a click on it (or on its speech bubble) opens the chat in the notch itself. PetController.swift stays, unused.
    /// What an agent says on its own (PARLEY's picks), in a bubble under the circle.
    private lazy var speech = AgentSpeechBubble()
    private var speechSubscription: AnyCancellable?
    private var hideModeSubscription: AnyCancellable?
    private var frameTimer: Timer?
    private var keyMonitor: Any?
    private var viewSubscription: AnyCancellable?

    // Confused recovery timer (set by handleDizzy)
    private var confusedRecoveryTimer: DispatchWorkItem?

    // Finished-pin timer
    private var finishedPinTimer: DispatchWorkItem?

    // Bot-head hover (love emote — mirrors prototype botHover())
    private var hoverTimer: DispatchWorkItem?
    private var botHoverTimer: DispatchWorkItem?
    private var botHovering: Bool = false
    private var lastLoveTime: Double = 0
    private var botHoverStartPos: CGPoint = .zero

    // Window attach drag (M8)
    private var attachDragStart: NSPoint? = nil
    private var pendingIslandClick = false   // any island click → expand on mouseUp
    private var inAttachDrag = false
    private var dragGhostPanel: NSPanel? = nil
    private var dragGhostSize: CGFloat = 0
    private var ghostCurrentOrigin: NSPoint = .zero
    private var highlightPanel: NSPanel? = nil
    private var highlightWindowPid: pid_t = 0

    // Notch real dimensions (set on init)
    private var notchW: CGFloat = IslandConst.notchWidth
    private var notchH: CGFloat = IslandConst.notchHeight

    convenience init() {
        let screen = Self.notchScreen() ?? NSScreen.main!
        let nW = Self.notchWidth(for: screen)
        let nH = Self.notchHeight(for: screen)

        let panelW: CGFloat = 720
        let panelH: CGFloat = IslandConst.panelHeight
        let sf = screen.frame
        let panel = IslandPanel(
            contentRect: NSRect(x: sf.midX - panelW/2, y: sf.maxY - panelH,
                                width: panelW, height: panelH),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered, defer: false
        )
        panel.notchWidth  = nW
        panel.notchHeight = nH

        self.init(window: panel)
        self.islandPanel = panel
        self.notchW = nW
        self.notchH = nH
        setupPanel(screen: screen)
    }

    private func setupPanel(screen: NSScreen) {
        guard let panel = window as? IslandPanel else { return }
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = false
        panel.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.mainMenuWindow)) + 3)
        panel.collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .ignoresCycle]
        panel.ignoresMouseEvents = true

        // Propagate real notch dimensions to AppState
        AppState.shared.notchWidth  = notchW
        AppState.shared.notchHeight = notchH

        let contentSize = panel.contentRect(forFrameRect: panel.frame).size

        // Apple-recommended pattern: put NSHostingView and drag destination as siblings
        // inside a common superview, rather than embedding one inside the other.
        let container = NSView(frame: NSRect(origin: .zero, size: contentSize))
        container.autoresizingMask = [.width, .height]

        let hosting = NSHostingView(rootView: IslandRootView().environmentObject(AppState.shared))
        hosting.frame = NSRect(origin: .zero, size: contentSize)
        hosting.autoresizingMask = [.width, .height]

        // FileDropNSView sits below the hosting view (hitTest returns nil → no mouse interference).
        // AppKit routes NSDraggingDestination events to registered views independently of hitTest.
        let dropView = FileDropNSView(frame: NSRect(origin: .zero, size: contentSize))
        dropView.autoresizingMask = [.width, .height]
        dropView.onDragEntered = { [weak self] loc in
            Task { @MainActor in
                let iLoc = self?.windowToIsland(loc) ?? CGPoint(x: 320, y: 88)
                AppState.shared.fileDragOver = true
                // An open chat takes the file as its attachment: the composer lights up, no upload scene.
                if AppState.shared.view == .prompt { return }
                // enterZone sets isActive=true BEFORE hookExpand triggers re-render,
                // so IslandContainer sees isActive=true when state.view becomes .upload.
                UploadSequenceEngine.shared.enterZone(x: iLoc.x, y: iLoc.y)
                NotificationCenter.default.post(name: .hookExpand, object: IslandView.upload)
                NotificationCenter.default.post(name: .botMorphTo, object: CGFloat(1))
            }
        }
        dropView.onDragUpdated = { [weak self] loc in
            Task { @MainActor in
                if AppState.shared.view == .prompt { return }
                let iLoc = self?.windowToIsland(loc) ?? CGPoint(x: 320, y: 88)
                UploadSequenceEngine.shared.updateCursor(x: iLoc.x, y: iLoc.y)
            }
        }
        dropView.onDragExited = {
            Task { @MainActor in
                AppState.shared.fileDragOver = false
                if AppState.shared.view == .prompt { return }
                // Do NOT collapse — drag session still active; island stays open.
                NotificationCenter.default.post(name: .botMorphTo, object: CGFloat(0))
                UploadSequenceEngine.shared.exitZone()
            }
        }
        dropView.onFilesDropped = { urls in
            Task { @MainActor in
                await FileDropHandler.handle(urls: urls, state: AppState.shared)
            }
        }

        container.addSubview(hosting)    // z-bottom: SwiftUI + mouse events
        container.addSubview(dropView)   // z-top: drag only (hitTest→nil, transparent to mouse)
        panel.contentView = container

        startPolling()
        startKeyMonitor()
        wireFSM()
        startTips()

        // Make panel key whenever the prompt/chat view becomes active
        // (nonactivatingPanel never auto-becomes key, but TextField needs it)
        viewSubscription = state.$view
            .receive(on: DispatchQueue.main)
            .sink { [weak self] newView in
                guard let self else { return }
                if newView == .prompt {
                    self.islandPanel.makeKey()
                }
            }
    }

    // MARK: - Tips

    private var soundSubscription: AnyCancellable?

    private func startTips() {
        // Muting cuts what is playing at once, tips and ambient sounds included.
        soundSubscription = state.$soundEnabled.removeDuplicates().sink { on in
            if !on { SoundEngine.shared.stopAll() }
        }
        tips.screen = { [weak self] in self?.window?.screen }
        // Never over the island, a pending request, a drag or the greeting.
        tips.canShow = { [weak self] in
            guard let self else { return false }
            return self.state.mode != .expanded && self.fsm.state != .hello && !self.wasInIsland
                && self.state.pendingApproval == nil && !self.inAttachDrag && self.state.isPresent
        }
        tips.start()

        satellite.screen = { [weak self] in self?.window?.screen }
        satellite.start()
        usageBubble.screen = { [weak self] in self?.window?.screen }
        usageBubble.start()
        UsageRefresher.shared.start()
        satellite.onTap = { [weak self] in self?.openChatInNotch(agent: nil) }
        // An agent that has something to say (PARLEY found a pick) says it beside the circle, when the circle is what is
        // on screen and nothing else is open there. The LED board says it too, except Telegram's (AppState.agentSays).
        speech.onOpen = { [weak self] agent in self?.openChatInNotch(agent: agent) }
        state.onAgentSays = { [weak self] said in
            guard let self, self.state.mode == .hidden, self.state.hideMode == .bubble,
                  let circle = self.satellite.hoverRect, let screen = self.window?.screen?.frame else { return }
            self.speech.show(said, under: circle, on: screen)
        }
        speechSubscription = state.$mode.removeDuplicates()
            .sink { [weak self] mode in if mode != .hidden { self?.speech.dismiss(animated: false) } }

        // "Mantener el notch": the compact island never hides by itself. The other modes hide it after a minute.
        hideModeSubscription = state.$hideMode.removeDuplicates().sink { [weak self] mode in
            guard let self else { return }
            self.fsm.petitToHiddenDelay = mode == .notch ? .infinity : 60
            if self.fsm.state == .petit && !self.wasInIsland { self.fsm.idle() }   // re-arm (or drop) the hide timer
        }
    }

    // MARK: - FSM wiring

    private func wireFSM() {
        fsm.onTransition = { [weak self] from, to in
            guard let self else { return }
            switch to {
            case .hidden:
                self.setMode(.hidden)

            case .petit:
                if from == .hello {
                    // Fire interrupt first so canvas collapse starts before mode change
                    NotificationCenter.default.post(name: .greetingInterrupt, object: nil)
                } else if from == .hidden {
                    SoundEngine.shared.play("peek")
                }
                // setMode BEFORE changing view: onChange(of: state.view) guards on .expanded,
                // so setting view while already compact won't trigger a spurious open animation.
                self.setMode(.compact)
                if from == .hello { self.state.view = self.defaultView() }
                // Cursor elsewhere: nothing will fire mouseLeft, so arm the hide timer
                if !self.wasInIsland { self.fsm.idle() }

            case .home:
                // Back from a collapse: the chat the user left is still there.
                self.expand(to: reopenView(current: self.state.view, hasTasks: !self.state.tasks.isEmpty))
                // Opened with the cursor elsewhere (not by hovering): arm the idle timer
                if !self.wasInIsland { self.fsm.idle() }

            case .hello:
                self.expand(to: .greeting)
            }
        }

        // FSM observes greetComplete notification
        NotificationCenter.default.addObserver(
            forName: .greetComplete, object: nil, queue: .main
        ) { [weak self] _ in
            self?.fsm.greetComplete()
        }
    }

    // MARK: - 60 Hz polling loop

    private func startPolling() {
        frameTimer = Timer.scheduledTimer(withTimeInterval: 1.0/60.0, repeats: true) { [weak self] _ in
            guard let self else { return }
            Task { @MainActor in self.pollFrame() }
        }
        RunLoop.main.add(frameTimer!, forMode: .common)
    }

    private func pollFrame() {
        guard let panel = window as? IslandPanel else { return }

        let mouse = NSEvent.mouseLocation

        // Convert mouse to panel-local coords (macOS: origin bottom-left)
        let pf = panel.frame
        let local = CGPoint(x: mouse.x - pf.minX, y: mouse.y - pf.minY)

        // Island rect in panel coords
        let islandRect = panel.currentIslandFrame(nw: notchW, nh: notchH)
        // While the island is hidden, the circle beside the notch takes clicks only while the cursor is over it
        // a click opens the chat in the notch.
        let circle = state.mode == .hidden ? satellite.hoverRect : nil
        satellite.setInteractive(circle?.insetBy(dx: -4, dy: -4).contains(mouse) ?? false)
        let inIsland   = islandRect.insetBy(dx: -6, dy: -6).contains(local)

        // Toggle click-through
        let shouldAcceptMouse = inIsland || inAttachDrag || attachDragStart != nil
        if panel.ignoresMouseEvents == shouldAcceptMouse {
            panel.ignoresMouseEvents = !shouldAcceptMouse
            if shouldAcceptMouse, let cv = panel.contentView {
                panel.invalidateCursorRects(for: cv)
            }
        }

        // Mouse in screen coords (Y flipped, origin top-left) for Bot look-at
        let screenH = panel.screen?.frame.height ?? NSScreen.main!.frame.height
        let newPos = CGPoint(x: mouse.x - (panel.screen?.frame.minX ?? 0), y: screenH - mouse.y)
        let cur = AppState.shared.mousePosition
        if abs(newPos.x - cur.x) > 1 || abs(newPos.y - cur.y) > 1 {
            AppState.shared.mousePosition = newPos
        }

        // Feed FSM hover enter/leave. wasInIsland is updated first: the FSM's
        // transitions ask whether the cursor is over the island.
        let entered = inIsland && !wasInIsland
        let left = !inIsland && wasInIsland
        wasInIsland = inIsland
        // The auto-close countdown only means something while the idle timer is running,
        // i.e. with the cursor away. Hovering the island is activity, so the bar stays down.
        if inIsland { state.lastActivity = .now }
        if entered {
            guard !inAttachDrag else { return }
            // If in mika: tell greeting to stay open (tc → infinity)
            if fsm.state == .hello {
                NotificationCenter.default.post(name: .greetingHover, object: nil)
            }
            fsm.mouseEntered()
        }
        if left { leaveIsland() }

        // Bot-head hover (love emote)
        let overBot = state.mode == .expanded && state.stateOverride == nil && isBotHit(local)
        if overBot && !botHovering { botHoverIn(mousePos: NSEvent.mouseLocation) }
        if !overBot && botHovering { botHoverOut() }
        botHovering = overBot
        if botHovering {
            let m = NSEvent.mouseLocation
            let dist = hypot(m.x - botHoverStartPos.x, m.y - botHoverStartPos.y)
            if dist > 40 {
                botHoverStartPos = m
                botHoverTimer?.cancel()
                scheduleLoveTimer()
            }
        }

        // Ghost Mika follows cursor + window highlight during drag (60 Hz, no throttle)
        if inAttachDrag {
            updateDragGhost()
            updateWindowHighlight()
        }
    }

    private var lastMouse: CGPoint = .zero

    /// What the FSM must not close the island over: a permission request still waiting
    /// for its answer, or a window-attach drag under way. Called before every cursor
    /// event and before arming the idle timer, so the flags are never stale.
    private func syncFSMGuards() {
        let decision = state.isPinned || state.isPickingFile || state.view == .approval || state.view == .question
        fsm.holdOpen = decision || inAttachDrag || attachDragStart != nil
        // The idle timer also leaves alone the views the user works in.
        fsm.pinned = fsm.holdOpen || state.view == .error || state.view == .prompt
    }

    /// The cursor left the island: close it now, unless something is holding it open.
    private func leaveIsland() {
        syncFSMGuards()
        fsm.mouseLeft()
    }

    // MARK: - Bot-head hover (love emote — mirrors prototype botHover())

    private func botHoverIn(mousePos: CGPoint) {
        guard state.mode == .expanded, state.stateOverride == nil else { return }
        guard CACurrentMediaTime() - lastLoveTime > 6 else { return }
        botHoverStartPos = mousePos
        NotificationCenter.default.post(name: .botBlink, object: nil)
        NotificationCenter.default.post(name: .botSetTgEs, object: CGFloat(1.08))
        SoundEngine.shared.play("hover")
        scheduleLoveTimer()
    }

    private func botHoverOut() {
        botHoverTimer?.cancel()
        NotificationCenter.default.post(name: .botSetTgEs, object: CGFloat(1))
    }

    private func scheduleLoveTimer() {
        botHoverTimer?.cancel()
        let item = DispatchWorkItem { [weak self] in
            guard let self, self.botHovering, self.state.stateOverride == nil else { return }
            guard CACurrentMediaTime() - self.lastLoveTime > 6 else { return }
            self.lastLoveTime = CACurrentMediaTime()
            NotificationCenter.default.post(name: .triggerEmote, object: BotEmote.love)
            SoundEngine.shared.play("love")
        }
        botHoverTimer = item
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.9, execute: item)
    }

    private func scheduleHover(after delay: TimeInterval, action: @escaping () -> Void) {
        hoverTimer?.cancel()
        let item = DispatchWorkItem(block: action)
        hoverTimer = item
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: item)
    }

    // MARK: - Mode transitions

    private func modeLevel(_ m: IslandMode) -> Int {
        switch m { case .hidden: return 0; case .compact: return 1; case .expanded: return 2 }
    }

    func setMode(_ mode: IslandMode) {
        let prev = state.mode
        guard mode != prev else { return }
        let shrinking = modeLevel(mode) < modeLevel(prev)
        let anim: Animation = shrinking
            ? .timingCurve(0.45, 0, 0.2, 1, duration: 0.34)
            : .spring(response: 0.5, dampingFraction: 0.72)
        withAnimation(anim) { state.mode = mode }
        if mode == .expanded {
            // A notice still unanswered comes back once the island closes again.
            let unanswered = tips.showingNotice ? pendingNotice : nil
            tips.dismiss(animated: false)
            SoundEngine.shared.play("open")
            if let n = unanswered { presentNotice(agent: n.agent, text: n.text, action: n.action, path: n.path, tries: 0) }
        }
        if prev == .expanded { SoundEngine.shared.play("close"); state.isPinned = false }
    }

    /// The notice on screen and not answered yet.
    private var pendingNotice: (agent: String, text: String, action: String, path: String?)?

    private func presentNotice(agent: String, text: String, action: String, path: String?, tries: Int) {
        guard let task = state.tasks.first(where: { $0.id == AgentTask.agentTaskID(agent) }) else { return }
        if state.mode == .expanded || state.pendingApproval != nil {
            guard tries < 300 else { return }
            DispatchQueue.main.asyncAfter(deadline: .now() + 2) { [weak self] in
                self?.presentNotice(agent: agent, text: text, action: action, path: path, tries: tries + 1)
            }
            return
        }
        pendingNotice = (agent, text, action, path)
        tips.notice(task: task, text: text, action: action, onAction: { [weak self] in
            if let path { Notifier.openDigest(path: path) } else { self?.openChatInNotch(agent: agent) }
        }, onDone: { [weak self] in self?.pendingNotice = nil })
    }

    /// The chat of `agent` (the main one when nil) in the notch: the only chat there is on the Mac.
    func openChatInNotch(agent: String?) {
        let id = agent ?? state.hub.activeID
        state.openChat(agentID: id)
        expand(to: .prompt)
    }

    func expand(to view: IslandView) {
        fsm.adoptHome()   // alert, hotkey, menu item or drop opened us: keep the FSM in step
        state.view = view
        // Opened with the cursor elsewhere: nothing will fire mouseLeft, so arm the idle timer.
        if !wasInIsland {
            syncFSMGuards()
            fsm.idle()
        }
        if state.mode == .expanded {
            // Already expanded — just switch view
        } else {
            setMode(.expanded)
        }
        state.lastActivity = .now
    }

    func collapse() {
        state.isPinned = false
        finishedPinTimer?.cancel()
        // Drive the FSM rather than only the mode: a stale "home" there made the next
        // hover think the island was already open.
        fsm.forcePetit()
        setMode(.compact)   // no-op when the FSM has already moved us there
        window?.resignKey()
    }

    // MARK: - Keyboard (Escape closes)

    private func startKeyMonitor() {
        keyMonitor = NSEvent.addGlobalMonitorForEvents(matching: .keyDown) { [weak self] event in
            Task { @MainActor in
                guard let self = self else { return }
                if event.keyCode == 53 { // Escape
                    if self.state.mode == .expanded && !self.state.isPinned {
                        self.collapse()
                    }
                }
            }
        }

        // Hook server expand requests (alerts only)
        NotificationCenter.default.addObserver(forName: .hookExpand, object: nil, queue: .main) { [weak self] note in
            guard let self, let view = note.object as? IslandView else { return }
            self.expand(to: view)
        }

        // Hook server compact reveal (non-alert work events: session start, tool use, etc.)
        NotificationCenter.default.addObserver(forName: .hookReveal, object: nil, queue: .main) { [weak self] _ in
            guard let self else { return }
            self.fsm.reveal()
        }

        // PARLEY has a real pick: the island comes out on the Telegram card, as the greeting does at launch. Never over what the
        // user is doing (the island already open, a request waiting, a drag).
        NotificationCenter.default.addObserver(forName: .agentGreets, object: nil, queue: .main) { [weak self] note in
            guard let self, self.state.mode != .expanded, self.state.pendingApproval == nil, !self.inAttachDrag,
                  self.fsm.state != .hello, let id = note.userInfo?["id"] as? String else { return }
            self.state.setFocus(id)
            self.expand(to: self.state.tasks.isEmpty ? .empty : .overview)
        }

        // An agent asks something small in the notch (PARLEY's analysis is ready): never over the open island; it waits
        // until the island closes (up to ten minutes), then the bubble with its button comes out.
        NotificationCenter.default.addObserver(forName: .agentNotice, object: nil, queue: .main) { [weak self] note in
            guard let info = note.userInfo, let agent = info["agent"] as? String, let text = info["text"] as? String,
                  let action = info["action"] as? String else { return }
            let path = info["path"] as? String
            MainActor.assumeIsolated { self?.presentNotice(agent: agent, text: text, action: action, path: path, tries: 0) }
        }

        // Collapse requests from views (OK button, etc.)
        NotificationCenter.default.addObserver(forName: .islandCollapse, object: nil, queue: .main) { [weak self] _ in
            self?.collapse()
        }

        // .botDizzy — posted by BotEngine.slap() on 3rd hit; show confused view + recover after 3.3s
        NotificationCenter.default.addObserver(forName: .botDizzy, object: nil, queue: .main) { [weak self] _ in
            self?.handleDizzy()
        }

        // Window attach drag.
        // Uses MainActor.assumeIsolated (synchronous) to avoid race with pollFrame().
        // Global mouseUp is the reliable fallback when cursor is outside our panel frame.
        NSEvent.addLocalMonitorForEvents(matching: .leftMouseDown) { [weak self] event in
            guard let self else { return event }
            MainActor.assumeIsolated {
                guard self.wasInIsland else { return }
                self.pendingIslandClick = true
                self.hoverTimer?.cancel()
                self.botHoverTimer?.cancel()
                self.botHovering = false
                // Drag only starts when clicking directly on the bot head
                guard self.isBotHit(event.locationInWindow) else { return }
                self.attachDragStart = NSEvent.mouseLocation
                // Post slap only when expanded
                guard self.state.mode == .expanded else { return }
                NotificationCenter.default.post(name: .triggerSlap, object: nil)
            }
            return event
        }
        NSEvent.addLocalMonitorForEvents(matching: .leftMouseDragged) { [weak self] event in
            guard let self else { return event }
            MainActor.assumeIsolated {
                guard let start = self.attachDragStart, !self.inAttachDrag else { return }
                let m = NSEvent.mouseLocation
                guard hypot(m.x - start.x, m.y - start.y) > 3 else { return }
                self.inAttachDrag = true
                NotificationCenter.default.post(name: .triggerEmote, object: BotEmote.love)
                self.showDragGhost()
            }
            return event
        }

        // mouseUp — local (cursor still in panel) + global (cursor moved outside panel frame)
        let finishDrag: @Sendable () -> Void = { [weak self] in
            Task { @MainActor in
                guard let self, self.inAttachDrag else { return }
                let mouse = NSEvent.mouseLocation
                self.inAttachDrag = false
                self.attachDragStart = nil
                self.state.stateOverride = nil
                self.hideDragGhost()
                if let ctx = self.windowContextAtPoint(mouse) {
                    self.state.promptContext = ctx
                    SoundEngine.shared.play("approve")
                    NotificationCenter.default.post(name: .triggerEmote, object: BotEmote.happy)
                    self.expand(to: .prompt)
                } else if !self.wasInIsland {
                    // Dropped on nothing, outside: the cursor already left, so close now.
                    self.leaveIsland()
                }
            }
        }
        NSEvent.addLocalMonitorForEvents(matching: .leftMouseUp) { [weak self] event in
            guard let self else { return event }
            MainActor.assumeIsolated {
                let hadPendingClick = self.pendingIslandClick
                let wasDragging     = self.inAttachDrag
                self.pendingIslandClick = false
                if wasDragging {
                    finishDrag()
                } else {
                    self.attachDragStart = nil
                    if hadPendingClick && self.state.mode != .expanded {
                        self.fsm.click()   // FSM petit→home; onTransition calls expand(to:)
                    }
                }
            }
            return event
        }
        NSEvent.addGlobalMonitorForEvents(matching: .leftMouseUp) { _ in
            finishDrag()
        }

        // Global hotkey to show island
        NSEvent.addGlobalMonitorForEvents(matching: .keyDown) { [weak self] event in
            Task { @MainActor in
                guard let self, self.state.hotkeyEnabled else { return }
                let pressed = event.modifierFlags.intersection([.command, .control, .option, .shift]).rawValue
                guard pressed == self.state.hotkeyFlags, event.keyCode == self.state.hotkeyCode else { return }
                if self.state.mode == .hidden || self.state.mode == .compact {
                    self.expand(to: .overview)
                }
            }
        }

        // Track last external app for window context capture
        let ourBundle = Bundle.main.bundleIdentifier ?? ""
        NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didActivateApplicationNotification,
            object: nil, queue: .main
        ) { [weak self] note in
            guard let self else { return }
            if let app = note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication,
               app.bundleIdentifier != ourBundle {
                self.state.lastExternalApp = app
            }
        }
    }

    // MARK: - Drag ghost window (Mika follows cursor during drag)

    private func showDragGhost() {
        guard dragGhostPanel == nil else { return }
        // Same size as compact bot: diameter=20 → canvasSize≈33, scale 2× for grab comfort
        let canvasSize: CGFloat = 40 / 0.6      // ~67
        dragGhostSize = canvasSize

        let mouse = NSEvent.mouseLocation
        let s = dragGhostSize
        ghostCurrentOrigin = NSPoint(x: mouse.x - s/2, y: mouse.y - s/2)

        let panel = NSPanel(
            contentRect: NSRect(x: ghostCurrentOrigin.x, y: ghostCurrentOrigin.y, width: s, height: s),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = false
        panel.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.mainMenuWindow)) + 4)
        panel.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle]
        panel.ignoresMouseEvents = true

        let hosting = NSHostingView(
            rootView: GhostBotView(canvasSize: canvasSize)
        )
        hosting.frame = NSRect(x: 0, y: 0, width: s, height: s)
        panel.contentView = hosting
        panel.alphaValue = 0
        panel.orderFront(nil)
        dragGhostPanel = panel
        AppState.shared.isDraggingBot = true

        // Fade + scale-in handled by GhostBotView SwiftUI animation;
        // also fade in the window itself for extra smoothness
        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = 0.18
            ctx.timingFunction = CAMediaTimingFunction(name: .easeOut)
            panel.animator().alphaValue = 1
        }
    }

    private func hideDragGhost() {
        dragGhostPanel?.close()
        dragGhostPanel = nil
        highlightPanel?.close()
        highlightPanel = nil
        highlightWindowPid = 0
        AppState.shared.isDraggingBot = false
    }

    private func updateDragGhost() {
        guard let panel = dragGhostPanel else { return }
        let s = dragGhostSize
        let mouse = NSEvent.mouseLocation
        // Direct follow — bot is "held", no trailing lag
        ghostCurrentOrigin = NSPoint(x: mouse.x - s/2, y: mouse.y - s/2)
        panel.setFrameOrigin(ghostCurrentOrigin)
    }

    // MARK: - Window highlight overlay (white border on target window during drag)

    private func updateWindowHighlight() {
        let mouse = NSEvent.mouseLocation
        guard let (appKitBounds, pid) = windowBoundsAtScreenPoint(mouse) else {
            // Fade out + close if no window under cursor
            if let old = highlightPanel {
                let captured = old
                highlightPanel = nil
                highlightWindowPid = 0
                NSAnimationContext.runAnimationGroup({ ctx in
                    ctx.duration = 0.12
                    ctx.timingFunction = CAMediaTimingFunction(name: .easeIn)
                    captured.animator().alphaValue = 0
                }, completionHandler: { captured.close() })
            }
            return
        }

        if pid == highlightWindowPid, let existing = highlightPanel {
            // Same window — just track position (windows rarely move, instant is fine)
            existing.setFrame(appKitBounds, display: false)
        } else {
            // New window — close old immediately, fade-in new
            highlightPanel?.close()
            highlightPanel = nil
            highlightWindowPid = pid

            let panel = NSPanel(
                contentRect: appKitBounds,
                styleMask: [.borderless, .nonactivatingPanel],
                backing: .buffered, defer: false
            )
            panel.backgroundColor = .clear
            panel.isOpaque = false
            panel.hasShadow = false
            panel.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.mainMenuWindow)) + 2)
            panel.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle]
            panel.ignoresMouseEvents = true

            let hosting = NSHostingView(rootView:
                RoundedRectangle(cornerRadius: 12)
                    .stroke(Color.white.opacity(0.75), lineWidth: 3)
                    .shadow(color: Color.white.opacity(0.5), radius: 16)
                    .padding(2)
                    .ignoresSafeArea()
            )
            hosting.frame = CGRect(origin: .zero, size: appKitBounds.size)
            hosting.autoresizingMask = [.width, .height]
            panel.contentView = hosting
            panel.alphaValue = 0
            panel.orderFront(nil)
            highlightPanel = panel

            NSAnimationContext.runAnimationGroup { ctx in
                ctx.duration = 0.14
                ctx.timingFunction = CAMediaTimingFunction(name: .easeOut)
                panel.animator().alphaValue = 1
            }
        }
    }

    private func windowBoundsAtScreenPoint(_ screenPoint: NSPoint) -> (CGRect, pid_t)? {
        guard let screen = window?.screen ?? NSScreen.main else { return nil }
        let screenMaxY = screen.frame.maxY
        let cgPoint = CGPoint(x: screenPoint.x, y: screenMaxY - screenPoint.y)

        guard let list = CGWindowListCopyWindowInfo(
            [.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID
        ) as? [[String: Any]] else { return nil }

        let ourBundle = Bundle.main.bundleIdentifier ?? ""
        for info in list {
            guard let b = info[kCGWindowBounds as String] as? [String: Any],
                  let x = b["X"] as? CGFloat, let y = b["Y"] as? CGFloat,
                  let w = b["Width"] as? CGFloat, let h = b["Height"] as? CGFloat else { continue }
            guard CGRect(x: x, y: y, width: w, height: h).contains(cgPoint) else { continue }
            let pid = info[kCGWindowOwnerPID as String] as? pid_t ?? 0
            guard let app = NSRunningApplication(processIdentifier: pid),
                  app.bundleIdentifier != ourBundle,
                  app.activationPolicy == .regular else { continue }
            // CG → AppKit: flip Y
            return (CGRect(x: x, y: screenMaxY - y - h, width: w, height: h), pid)
        }
        return nil
    }

    // MARK: - Window context at screen point (for drag-attach)

    private func windowContextAtPoint(_ screenPoint: NSPoint) -> PromptContext? {
        let screen = window?.screen ?? NSScreen.main
        // CGWindowList uses top-left origin; NSEvent.mouseLocation uses bottom-left
        let screenMaxY = screen?.frame.maxY ?? NSScreen.main!.frame.maxY
        let cgPoint = CGPoint(x: screenPoint.x, y: screenMaxY - screenPoint.y)

        guard let windowList = CGWindowListCopyWindowInfo(
            [.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID
        ) as? [[String: Any]] else { return nil }

        let ourBundle = Bundle.main.bundleIdentifier ?? ""

        for info in windowList {
            guard let b = info[kCGWindowBounds as String] as? [String: Any],
                  let x = b["X"] as? CGFloat, let y = b["Y"] as? CGFloat,
                  let w = b["Width"] as? CGFloat, let h = b["Height"] as? CGFloat else { continue }
            guard CGRect(x: x, y: y, width: w, height: h).contains(cgPoint) else { continue }

            let pid = info[kCGWindowOwnerPID as String] as? pid_t ?? 0
            guard let app = NSRunningApplication(processIdentifier: pid),
                  app.bundleIdentifier != ourBundle,
                  app.activationPolicy == .regular else { continue }

            return WindowContextCapture.captureActive(from: app)
        }
        return nil
    }

    // MARK: - Coordinate conversion: window (AppKit, y-up) → island coords (y-down, 0,0 = island top-left)

    func windowToIsland(_ loc: CGPoint) -> CGPoint {
        let panelH = window?.frame.height ?? IslandConst.panelHeight
        let panelW = window?.frame.width  ?? 720
        let islandLeft = (panelW - IslandConst.expandedWidth) / 2
        // Island is glued to panel top; its bottom in AppKit = panelH - 176
        return CGPoint(
            x: loc.x - islandLeft,
            y: panelH - loc.y                // AppKit y is from bottom; island y from top
        )
    }

    // MARK: - Helpers

    func defaultView() -> IslandView {
        state.tasks.isEmpty ? .empty : .overview
    }

    func baseMode() -> IslandMode {
        guard state.isPresent else { return .hidden }
        return state.tasks.isEmpty ? .hidden : .compact
    }

    // MARK: - Activity reset (call on any user interaction in island)

    func resetActivity() {
        state.lastActivity = .now
    }

    // MARK: - Finished task pin (5.2s)

    func pinForFinished(taskId: String) {
        state.isPinned = true
        finishedPinTimer?.cancel()
        let item = DispatchWorkItem { [weak self] in
            guard let self else { return }
            self.state.removeTask(id: taskId)
            self.state.isPinned = false
            self.collapse()
        }
        finishedPinTimer = item
        DispatchQueue.main.asyncAfter(deadline: .now() + 5.2, execute: item)
    }

    // MARK: - Dizzy recovery (triggered by BotEngine.slap via .botDizzy)

    private func handleDizzy() {
        let prevView = state.view
        state.stateOverride = .dizzy
        expand(to: .confused)
        confusedRecoveryTimer?.cancel()
        let recovery = DispatchWorkItem { [weak self] in
            guard let self else { return }
            self.state.stateOverride = nil
            if self.state.view == .confused {
                let fallback = self.state.tasks.isEmpty ? IslandView.empty : .overview
                self.state.view = (prevView == .confused) ? fallback : prevView
            }
            NotificationCenter.default.post(name: .triggerEmote, object: BotEmote.happy)
        }
        confusedRecoveryTimer = recovery
        DispatchQueue.main.asyncAfter(deadline: .now() + 3.3, execute: recovery)
    }

    // MARK: - Bot hit test (for slap trigger)

    private func isBotHit(_ windowPoint: CGPoint) -> Bool {
        let s = AppState.shared
        let panelH = window?.frame.height ?? IslandConst.panelHeight
        let panelW = window?.frame.width  ?? 720
        let (islandW, fixedH) = islandSize(mode: s.mode, view: s.view,
                                            progress: s.uploadProgress, nw: notchW, nh: notchH,
                                            mediaExtra: s.mediaExtraHeight)
        // Chat view resizes dynamically — must match IslandContainer.chatPromptHeight
        let islandH: CGFloat
        if s.mode == .expanded && s.view == .prompt {
            islandH = IslandConst.chatPromptHeight(messages: s.chatHistory.count)
        } else {
            islandH = fixedH
        }
        let islandMinX = (panelW - islandW) / 2
        let (cx, cy, diameter, _) = botPosition(mode: s.mode, view: s.view,
                                                  islandW: islandW, islandH: islandH,
                                                  uploadProgress: s.uploadProgress,
                                                  mediaExtra: s.mediaExtraHeight)
        let radius = (diameter / 0.6) / 2
        // botPosition cy is from island TOP; panel AppKit coords have y=0 at bottom
        // island top in AppKit coords = panelH (island glued to top of panel/screen)
        let botX = islandMinX + cx
        let botY = panelH - cy
        let dx = windowPoint.x - botX
        let dy = windowPoint.y - botY
        return dx*dx + dy*dy <= radius * radius
    }

    // MARK: - Notch detection (static)

    static func notchScreen() -> NSScreen? {
        NSScreen.screens.first { $0.safeAreaInsets.top > 0 }
    }

    static func notchWidth(for screen: NSScreen) -> CGFloat {
        let aux = (screen.auxiliaryTopLeftArea?.width ?? 0) +
                  (screen.auxiliaryTopRightArea?.width ?? 0)
        let w = screen.frame.width - aux
        return w > 0 ? w : IslandConst.notchWidth
    }

    static func notchHeight(for screen: NSScreen) -> CGFloat {
        let h = screen.safeAreaInsets.top
        return h > 0 ? h : IslandConst.notchHeight
    }

    nonisolated func cleanup() {
        // Called explicitly before release if needed
    }
}

// MARK: - IslandPanel

final class IslandPanel: NSPanel {
    var notchWidth:  CGFloat = IslandConst.notchWidth
    var notchHeight: CGFloat = IslandConst.notchHeight

    override var canBecomeKey:  Bool { true }
    override var canBecomeMain: Bool { false }

    /// Allow panel to sit in the menu bar / notch area — don't let macOS push it down.
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect {
        return frameRect
    }

    func currentIslandFrame(nw: CGFloat, nh: CGFloat) -> CGRect {
        let s = AppState.shared
        let (w, fixedH) = islandSize(mode: s.mode, view: s.view,
                                      progress: s.uploadProgress, nw: nw, nh: nh,
                                      mediaExtra: s.mediaExtraHeight)
        let h: CGFloat
        if s.mode == .expanded && s.view == .prompt {
            h = IslandConst.chatPromptHeight(messages: s.chatHistory.count)
        } else {
            h = fixedH
        }
        return CGRect(x: (frame.width - w) / 2, y: frame.height - h, width: w, height: h)
    }
}

// MARK: - Ghost bot view (animated scale-in on appear)

struct GhostBotView: View {
    let canvasSize: CGFloat
    @State private var scale: CGFloat = 0.35

    var body: some View {
        BotCanvasView(state: AppState.shared)
            .frame(width: canvasSize, height: canvasSize)
            .scaleEffect(scale)
            .onAppear {
                withAnimation(.spring(response: 0.28, dampingFraction: 0.55)) {
                    scale = 1.0
                }
            }
    }
}

// MARK: - Notification names

extension Notification.Name {
    static let triggerEmote     = Notification.Name("mika.triggerEmote")
    static let triggerSlap      = Notification.Name("mika.triggerSlap")
    static let botDizzy         = Notification.Name("mika.botDizzy")
    static let botGreet         = Notification.Name("mika.botGreet")
    static let botBlink         = Notification.Name("mika.botBlink")
    static let botSetTgEs       = Notification.Name("mika.botSetTgEs")
    static let botGulp          = Notification.Name("mika.botGulp")
    static let botMorphTo       = Notification.Name("mika.botMorphTo")
    static let islandAction     = Notification.Name("mika.islandAction")
    static let islandCollapse   = Notification.Name("mika.islandCollapse")
    static let openFullSettings = Notification.Name("mika.openFullSettings")
    static let hookReveal       = Notification.Name("mika.hookReveal")
    /// An agent has something to show on arrival (PARLEY's pick): userInfo["id"] is the pill whose card to show.
    static let agentGreets      = Notification.Name("mika.agentGreets")
    /// An agent asks the user something small in the notch with one button ("¿Lo ves?" · Ver). userInfo: agent, text,
    /// action, path (PARLEY's page).
    static let agentNotice      = Notification.Name("mika.agentNotice")
    // Greeting ↔ IslandWindowController
    static let greetComplete    = Notification.Name("mika.greetComplete")
    static let greetingHover    = Notification.Name("mika.greetingHover")
    static let greetingInterrupt = Notification.Name("mika.greetingInterrupt")
}

// MARK: - islandSize (takes real notch dimensions)

func islandSize(mode: IslandMode, view: IslandView,
                progress: Double = 0,
                nw: CGFloat = IslandConst.notchWidth,
                nh: CGFloat = IslandConst.notchHeight,
                mediaExtra: CGFloat = 0) -> (CGFloat, CGFloat) {
    switch mode {
    case .hidden:   return (nw, nh)
    case .compact:  return (nw + 160, nh)
    case .expanded:
        let layout = IslandConst.viewLayouts[view]!
        // The overview grows by the media strip's height while it is shown (AppState.mediaExtraHeight).
        return (IslandConst.expandedWidth, layout.height + mediaExtra)
    }
}
