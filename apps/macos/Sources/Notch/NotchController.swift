import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// The island in the notch (or a pill at the top of a screen without one). The window is a fixed transparent area at
/// the top centre; it only takes the mouse inside the island's current shape, so everything around it stays clickable.
/// Nothing runs while the pointer is still and nothing is shown.
@MainActor
final class NotchController {
    private let core: BuddyCore
    private let system: NotchSystemMonitor
    private let model = NotchModel()
    private var geometry: NotchGeometry?
    private var panel: NotchPanel?
    private var host: NSHostingView<NotchView>?
    private var monitors: [Any] = []
    private var hoverWork: DispatchWorkItem?
    private var leaveWork: DispatchWorkItem?
    private let media = MediaWatcher()
    private var hooksConnected = false
    private var utilitiesTask: Task<Void, Never>?
    private var calendarRead: Task<Void, Never>?
    private var calendarGeneration = 0
    private var shelfRefreshedAt = Date.distantPast
    private let lockState = NotchLockState()
    private let unlockMonitor = NotchUnlockMonitor()
    private var lockWorkspaceObservers: [NSObjectProtocol] = []
    private var lockPanel: NotchLockPanel?
    private var lockHost: NSHostingView<NotchLockView>?
    private var lockSpace: NotchLockSpace?
    private var lockObservers: [NSObjectProtocol] = []

    /// Opens a chat by id next to Buddy.
    var onOpenChat: ((String) -> Void)?
    /// Files handed to Buddy from the drop zone (opens the chat with them).
    var onGiveFiles: (([URL]) -> Void)?
    var onGiveText: ((String) -> Void)?
    /// A question for Buddy, sent as a chat message («No reconozco este movimiento…»).
    var onAskBuddy: ((String) -> Void)?

    /// The most the island ever needs; the window keeps this size.
    static let canvas = CGSize(width: 640, height: 460)

    init(core: BuddyCore) {
        self.core = core
        self.system = NotchSystemMonitor(core: core)
    }

    func start() {
        geometry = NotchGeometry.current()
        guard let geometry else { return }
        refreshHooks()
        let panel = NotchPanel(contentRect: geometry.frame(for: CGSize(width: Self.canvas.width, height: geometry.notch.height + Self.canvas.height)),
                               styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.mainMenuWindow)) + 3)
        panel.collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .ignoresCycle]
        panel.ignoresMouseEvents = true
        panel.isReleasedWhenClosed = false
        let host = NSHostingView(rootView: view())
        host.sizingOptions = []
        panel.contentView = host
        panel.orderFrontRegardless()
        self.panel = panel
        self.host = host
        model.system = system
        AppServices.notchSystem = system
        system.onStatus = { [weak self] status in self?.model.showStatus(status) }
        system.start()
        media.onChange = { [weak self] track in
            self?.model.nowPlaying = track
            self?.model.nowPlayingAt = Date()
        }
        model.youtube = core.youtubeStatus()
        model.focus = core.focusStatus().running ? core.focusStatus() : nil
        watchPlayers()
        model.shortcuts = (try? core.shortcuts()) ?? []
        model.usage = core.usage()
        if let tools = try? core.notchTools() { model.acceptTools(tools) }
        observe()
        watchScreenLock()
        let handler: (NSEvent) -> Void = { [weak self] _ in MainActor.assumeIsolated { self?.pointerMoved() } }
        monitors.append(NSEvent.addGlobalMonitorForEvents(matching: [.mouseMoved, .leftMouseDragged], handler: handler) as Any)
        monitors.append(NSEvent.addLocalMonitorForEvents(matching: [.mouseMoved, .leftMouseDragged]) { event in
            handler(event)
            return event
        } as Any)
        monitors.append(NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            let handled = MainActor.assumeIsolated {
                guard let self, event.keyCode == 53, event.window === self.panel else { return false }
                self.model.collapse()
                return true
            }
            return handled ? nil : event
        } as Any)
        #if DEBUG
        // Explicit debug fixtures only; normal launches always display actual system readings.
        switch ProcessInfo.processInfo.environment["BUDDY_DEBUG_NOTCH"] {
        case "lock": DispatchQueue.main.asyncAfter(deadline: .now() + 1) { self.screenLocked() }
        case "unlock": DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
            self.screenLocked()
            DispatchQueue.main.asyncAfter(deadline: .now() + 1) { self.screenUnlocked() }
        }
        case "open": DispatchQueue.main.asyncAfter(deadline: .now() + 1) { self.debugPinned = true; self.model.setHovering(true) }
        case "drop": DispatchQueue.main.asyncAfter(deadline: .now() + 1) { self.model.dragging = true }
        case "volume": debugStatus(.init(kind: .volume, title: "Volumen", symbol: "speaker.wave.2.fill", level: 0.65))
        case "brightness": debugStatus(.init(kind: .brightness, title: "Pantalla", symbol: "sun.max.fill", level: 0.75))
        case "connection": debugStatus(.init(kind: .connection, title: "AirPods", symbol: "airpodspro", text: "Conectado", active: true))
        case "focus": debugStatus(.init(kind: .focus, title: "No molestar", symbol: "moon.fill", text: "On", active: true))
        case "notification": DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
            self.debugPinned = true
            self.model.show(.init(kind: .finished, agent: "buddy", title: "Tu tarea está lista", detail: "Aviso de prueba"))
        }
        default: break
        }
        #endif
        NotificationCenter.default.addObserver(forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.relocate() }
        }
    }

    #if DEBUG
    private func debugStatus(_ status: NotchStatus) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
            self.debugPinned = true
            self.model.showStatus(status, seconds: 60)
        }
    }
    #endif

    // MARK: Core events

    func stop() {
        for observer in lockObservers { DistributedNotificationCenter.default().removeObserver(observer) }
        lockObservers.removeAll()
        unlockMonitor.stop()
        unlockMonitor.onUnlock = nil
        for observer in lockWorkspaceObservers { NSWorkspace.shared.notificationCenter.removeObserver(observer) }
        lockWorkspaceObservers.removeAll()
        lockState.stop()
        lockState.onFinish = nil
        lockPanel?.close(); lockPanel = nil; lockHost = nil; lockSpace = nil
        system.stop()
        utilitiesTask?.cancel(); calendarRead?.cancel()
        media.stop()
        for monitor in monitors { NSEvent.removeMonitor(monitor) }
        monitors.removeAll()
        panel?.close()
    }

    /// Whether two video states differ at most in how far the video has played (and the caption of that moment).
    nonisolated static func sameButPosition(_ a: YouTubeStatus?, _ b: YouTubeStatus?) -> Bool {
        func still(_ status: YouTubeStatus?) -> YouTubeStatus? {
            guard var status else { return nil }
            status.detected?.seconds = 0; status.detected?.caption = ""
            status.viewer?.seconds = 0; status.viewer?.caption = ""
            return status
        }
        return still(a) == still(b)
    }

    func handle(_ event: Event) {
        switch event {
        case .youTubeChanged:
            let next = core.youtubeStatus()
            // A video playing in the browser reports its position every second. With the notch closed nothing on
            // screen shows it: the model is left alone, so nothing is drawn or measured again for it.
            if model.mode != .open, Self.sameButPosition(model.youtube, next) { break }
            let wasNotch = model.youtube?.viewer != nil && model.youtube?.destination == "notch"
            model.youtube = next
            if wasNotch, model.youtube?.destination == "floating" { model.collapse() }
            if !wasNotch, model.youtube?.viewer != nil, model.youtube?.destination == "notch" {
                model.tab = .home; model.pinned = true; model.setHovering(true)
            }
        case let .settingChanged(key):
            if key.hasPrefix("notch.system.") { system.reload() }
        case let .approvalRequest(requestId, sessionId, agent, project, title, summary, detail, canAllow, always):
            approvalSessions[requestId] = sessionId
            let name = AgentNames.name(agent)
            model.show(.init(kind: .approval(requestID: requestId, canAllow: canAllow), agent: agent,
                             title: agent == "buddy" ? "Buddy quiere \(title.prefix(1).lowercased() + title.dropFirst())" : "\(name) pide permiso en \(project)",
                             detail: "\(title): \(summary)", command: detail, always: always))
            NSSound(named: "Tink")?.play()
        case let .approvalClosed(requestId):
            approvalSessions[requestId] = nil
            model.closeApproval(requestId)
        case .usageChanged:
            model.usage = core.usage()
        case let .chatActivity(_, kind, label):
            model.buddyActivity = (kind, label)
        case .chatDone, .chatFailed:
            model.buddyActivity = nil
        case let .financeRecorded(monto, _, tipo, concepto, comercio, enlace):
            let what = comercio.isEmpty ? concepto : comercio
            if enlace.isEmpty {
                model.show(.init(kind: .finished, agent: "niko", title: "Niko anotó: \(monto) · \(what)",
                                 detail: concepto.isEmpty || comercio.isEmpty ? tipo.capitalized : "\(tipo.capitalized) · \(concepto)"))
            } else {
                // Found in the mail a moment ago: the card opens, asks, and links to that mail.
                let incoming = tipo == "ingreso" || tipo == "transferencia recibida"
                model.show(.init(kind: .finished, agent: "niko", title: "\(monto) · \(what)",
                                 detail: incoming ? "\(tipo.capitalized). Niko lo anotó." : "\(tipo.capitalized)\(concepto.isEmpty || comercio.isEmpty ? "" : " · \(concepto)"). ¿Fuiste tú?",
                                 link: enlace,
                                 ask: "No reconozco este movimiento que llegó a mi correo: \(monto) · \(what) (\(tipo)). ¿Qué debo hacer?"))
            }
        case let .budgetAlert(categoria, usadoPct):
            model.show(.init(kind: .waiting, agent: "niko",
                             title: usadoPct >= 100 ? "Te pasaste del presupuesto de \(categoria.capitalized)" : "Te queda \(100 - Int(usadoPct)) % en \(categoria.capitalized)",
                             detail: "Llevas \(usadoPct) % del tope del mes. Pregúntale a Niko en qué se fue."))
        case let .mediaCommand(action, uri):
            runMediaCommand(action: action, uri: uri)
        case let .usageLow(provider, label, leftPct):
            let name = AgentNames.plan(provider)
            model.show(.init(kind: .waiting, agent: provider, title: "Te queda \(leftPct) % de \(name)",
                             detail: "Ventana: \(label). Buddy usará el otro proveedor si se acaba."))
        case let .mascotState(state):
            model.buddyBusy = state == "think" || state == "work"
            if !model.buddyBusy { model.buddyActivity = nil }
        case let .focusChanged(running, _):
            model.focus = running ? core.focusStatus() : nil
        case let .focusFinished(minutes):
            model.focus = nil
            model.show(.init(kind: .finished, agent: "buddy", title: "Terminó tu bloque de enfoque",
                             detail: "\(minutes) minutos. Tómate un respiro."))
            NSSound(named: "Glass")?.play()
        case let .sessionUpdate(sessionId, agent, project, state, cwd, terminal, summary):
            let previous = model.sessions.first { $0.id == sessionId }?.state
            model.update(session: sessionId, agent: agent, project: project, state: state)
            guard previous != state else { break }
            let name = AgentNames.name(agent)
            let place = cwd.isEmpty && terminal.isEmpty ? nil : NotchModel.Place(cwd: cwd, terminal: terminal)
            switch state {
            case "done":
                // What it said last when the agent tells us (Claude Code and Codex send it on Stop; agy does not), else
                // the project.
                model.show(.init(kind: .finished, agent: agent, title: "\(name) terminó en \(project)",
                                 detail: summary.isEmpty ? "Toca para volver a la sesión." : summary, place: place))
            // A permission request also puts the session in "waiting": its own card says it better.
            case "waiting":
                // The core sends the request a moment after this state: wait for it before saying anything.
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { [weak self] in
                    guard let self, !self.hasPendingApproval(for: sessionId),
                          self.model.sessions.first(where: { $0.id == sessionId })?.state == "waiting" else { return }
                    // A question the agent asked (Codex's question tool) comes with its text and options.
                    let asks = !summary.isEmpty
                    self.model.show(.init(kind: .waiting, agent: agent, title: asks ? "\(name) te pregunta en \(project)" : "\(name) espera tu respuesta",
                                          detail: asks ? summary : project, place: place, question: asks))
                }
            case "error": model.show(.init(kind: .failed, agent: agent, title: "\(name) se detuvo por un error", detail: project, place: place))
            default: break
            }
        default:
            break
        }
    }

    /// Approvals on screen or queued, by session (the core sends the request just after the "waiting" state).
    private var approvalSessions: [String: String] = [:]

    private func hasPendingApproval(for session: String) -> Bool {
        approvalSessions.values.contains(session)
    }

    // MARK: Hover and hit-testing

    /// The island's rectangle on screen right now.
    private var islandRect: NSRect {
        guard let geometry else { return .zero }
        return geometry.frame(for: NotchLayout.size(model, notch: geometry.notch))
    }

    #if DEBUG
    private var debugPinned = false
    #endif

    private func pointerMoved() {
        #if DEBUG
        if debugPinned { return }
        #endif
        guard lockState.phase == .hidden, let panel, let geometry else { return }
        let p = NSEvent.mouseLocation
        // While idle, the notch itself (a little wider) is what opens the island.
        // The pointer at the very top of the screen sits exactly on the rectangle's upper edge, which `contains` leaves
        // out: it flickered in and out there, opening and closing the island. The areas reach past the top edge, and
        // once open the island gets a margin so the pointer hugging its border does not close it.
        let compactStatus = model.status != nil || model.notice != nil
        let trigger = model.mode == .idle && !compactStatus ? geometry.frame(for: geometry.notch).insetBy(dx: -6, dy: -2) : islandRect.insetBy(dx: -10, dy: -10)
        let inside = NSRect(x: trigger.minX, y: trigger.minY, width: trigger.width, height: trigger.height + 30).contains(p)
        panel.ignoresMouseEvents = !inside
        if inside {
            leaveWork?.cancel(); leaveWork = nil
            guard !model.hovering, hoverWork == nil else { return }
            let work = DispatchWorkItem { [weak self] in
                guard let self else { return }
                self.hoverWork = nil
                self.refreshHooks()
                self.core.refreshUsage()
                self.model.setHovering(true)
            }
            hoverWork = work
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.12, execute: work)
        } else {
            hoverWork?.cancel(); hoverWork = nil
            guard model.hovering, leaveWork == nil else { return }
            let work = DispatchWorkItem { [weak self] in
                self?.leaveWork = nil
                self?.model.setHovering(false)
            }
            leaveWork = work
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
        }
    }

    // MARK: Screen lock

    private func watchScreenLock() {
        let center = DistributedNotificationCenter.default()
        for (name, locked) in [("com.apple.screenIsLocked", true), ("com.apple.screenIsUnlocked", false)] {
            lockObservers.append(center.addObserver(forName: Notification.Name(name), object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated {
                    if locked {
                        self?.screenLocked()
                        self?.unlockMonitor.start()
                    } else { self?.screenUnlocked() }
                }
            })
        }
        unlockMonitor.onUnlock = { [weak self] in self?.screenUnlocked() }
        let workspace = NSWorkspace.shared.notificationCenter
        lockWorkspaceObservers.append(workspace.addObserver(forName: NSWorkspace.screensDidSleepNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.unlockMonitor.stop() }
        })
        lockWorkspaceObservers.append(workspace.addObserver(forName: NSWorkspace.screensDidWakeNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, self.lockState.phase == .locked else { return }
                self.unlockMonitor.start()
            }
        })
        lockState.onFinish = { [weak self] in
            guard let self else { return }
            self.lockPanel?.orderOut(nil)
            self.panel?.orderFrontRegardless()
            self.pointerMoved()
        }
    }

    private func screenLocked() {
        hoverWork?.cancel(); hoverWork = nil
        leaveWork?.cancel(); leaveWork = nil
        core.youtubeClose()
        model.pinned = false
        model.dragging = false
        model.setHovering(false)
        lockState.lock()
        panel?.orderOut(nil)
        guard let geometry else { return }
        if lockPanel == nil {
            let panel = NotchLockPanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
            panel.isOpaque = false
            panel.backgroundColor = .clear
            panel.hasShadow = false
            panel.ignoresMouseEvents = true
            panel.hidesOnDeactivate = false
            panel.canBecomeVisibleWithoutLogin = true
            panel.isReleasedWhenClosed = false
            panel.level = NSWindow.Level(rawValue: Int(CGShieldingWindowLevel()))
            panel.collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .ignoresCycle]
            let host = NSHostingView(rootView: NotchLockView(state: lockState, notch: geometry.notch))
            host.sizingOptions = []
            panel.contentView = host
            lockPanel = panel
            lockHost = host
            lockSpace = NotchLockSpace()
            if let lockSpace {
                lockSpace.attach(panel)
            } else {
                NSLog("Buddy: lock-screen space unavailable; unlock indicator remains available on desktop.")
            }
        }
        relocateLockPanel()
        lockPanel?.orderFrontRegardless()
    }

    private func screenUnlocked() {
        guard lockState.phase == .locked else { return }
        // Keep the existing loginwindow panel and geometry; start before any desktop work.
        unlockMonitor.stop()
        lockState.unlock()
        lockHost?.displayIfNeeded()
    }

    private func relocateLockPanel() {
        guard let geometry, let lockPanel else { return }
        lockPanel.setFrame(geometry.frame(for: CGSize(width: geometry.notch.width + 96, height: geometry.notch.height + 12)), display: true)
        lockHost?.rootView = NotchLockView(state: lockState, notch: geometry.notch)
    }

    // MARK: Drawing

    private func view() -> NotchView {
        NotchView(model: model, notch: geometry?.notch ?? CGSize(width: 190, height: 32), hasNotch: geometry?.hasNotch ?? false, hooksConnected: hooksConnected,
                  actions: NotchActions(
                      askBuddy: { [weak self] text in self?.onAskBuddy?(text) },
                      answer: { [weak self] id, allow in
                          self?.core.answerApproval(requestId: id, allow: allow)
                          self?.model.closeApproval(id)
                      },
                      answerAlways: { [weak self] id in
                          self?.core.answerApprovalAlways(requestId: id)
                          self?.model.closeApproval(id)
                      },
                      openPlace: { [weak self] place in self?.open(place) },
                      connect: { [weak self] in self?.connectHooks() },
                      media: { [weak self] action in self?.media.send(action) },
                      seek: { [weak self] ms in self?.media.seek(toMs: ms) },
                      youtubeOpen: { [weak self] video in
                          guard let self else { return }
                          do {
                              _ = try self.core.youtubeOpen(sourceId: video.sourceId, videoId: video.videoId)
                              self.model.toolMessage = ""
                          } catch { self.model.toolMessage = "El video cambió o el navegador se desconectó. Vuelve a intentarlo." }
                      },
                      focusStart: { [weak self] minutes in _ = self?.core.focusStart(minutes: minutes) },
                      focusStop: { [weak self] in self?.core.focusStop() },
                      openShortcut: { [weak self] item in self?.open(item) },
                      addShortcut: { [weak self] in self?.addShortcut() },
                      removeShortcut: { [weak self] item in
                          self?.model.shortcuts = (try? self?.core.removeShortcut(id: item.id)) ?? self?.model.shortcuts ?? []
                      },
                      giveToBuddy: { [weak self] in
                          guard let self else { return }
                          let files = self.model.dropped.filter { FileManager.default.fileExists(atPath: $0.path) }
                          self.onGiveFiles?(files)
                      },
                      share: { [weak self] in self?.share() },
                      copyPaths: { [weak self] in
                          guard let self else { return }
                          NSPasteboard.general.clearContents()
                          NSPasteboard.general.setString(self.model.dropped.map(\.path).joined(separator: "\n"), forType: .string)
                      },
                      drop: { [weak self] urls in
                          guard let self else { return }
                          self.model.dragging = false
                          if !urls.isEmpty { self.performTool { self.model.acceptTools(try self.core.notchAddFiles(paths: urls.map(\.path))); self.model.revealFiles() } }
                      },
                      close: { [weak self] in if self?.model.youtube?.destination == "notch" { self?.core.youtubeClose() }; self?.model.collapse() },
                      addFiles: { [weak self] in self?.pickShelfFiles() },
                      removeFile: { [weak self] path in self?.performTool { guard let self else { return }; self.model.acceptTools(try self.core.notchRemoveFile(path: path)) } },
                      openFile: { url in NSWorkspace.shared.activateFileViewerSelecting([url]) },
                      saveClipboard: { [weak self] in self?.saveClipboard() },
                      copyClip: { [weak self] clip in self?.copyClip(clip) },
                      giveClip: { [weak self] clip in self?.onGiveText?(clip.text) },
                      removeClip: { [weak self] id in self?.performTool { guard let self else { return }; self.model.acceptTools(try self.core.notchRemoveClip(id: id)) } },
                      setWidget: { [weak self] widget, enabled in self?.performTool { guard let self else { return }; self.model.acceptTools(try self.core.notchWidget(widget: widget, enabled: enabled)); self.refreshUtilities() } },
                      pickCalendar: { [weak self] in self?.pickCalendar() },
                      clearCalendar: { [weak self] in self?.performTool { guard let self else { return }; self.model.acceptTools(try self.core.notchSetCalendar(path: nil)); self.model.appointment = nil; self.model.calendarError = ""; self.refreshUtilities() } },
                      openAppointment: { [weak self] in self?.openAppointment() }))
    }

    // MARK: Music for the ears

    /// Spotify and Music announce every change themselves: the ears follow those notices, no polling. One reading at
    /// launch (only if a player is already running) fills them until the first notice.
    private func watchPlayers() {
        let center = DistributedNotificationCenter.default()
        for (name, app) in [("com.spotify.client.PlaybackStateChanged", "Spotify"), ("com.apple.Music.playerInfo", "Música")] {
            center.addObserver(forName: Notification.Name(name), object: nil, queue: .main) { [weak self] note in
                let info = note.userInfo ?? [:]
                let state = info["Player State"] as? String ?? ""
                let title = info["Name"] as? String ?? ""
                let artist = info["Artist"] as? String ?? ""
                MainActor.assumeIsolated {
                    self?.model.earTrack = state.isEmpty || state == "Stopped" ? nil
                        : NotchModel.EarTrack(title: title, artist: artist, app: app, playing: state == "Playing")
                    self?.reportNowPlaying()
                }
            }
        }
        let players = MediaControl.runningPlayers()
        guard !players.isEmpty else { return }
        DispatchQueue.global(qos: .utility).async { [weak self] in
            let reading = MediaControl.read(players)
            Task { @MainActor in
                guard let self, self.model.earTrack == nil, let reading else { return }
                self.model.earTrack = .init(title: reading.title, artist: reading.artist, app: reading.app,
                                            playing: reading.status == .playing)
                self.reportNowPlaying()
            }
        }
    }

    // MARK: Music for Buddy's agents

    /// The core answers the agents' `now_playing` with this.
    private func reportNowPlaying() {
        core.setNowPlaying(now: model.earTrack.map {
            NowPlayingInfo(title: $0.title, artist: $0.artist, app: $0.app, playing: $0.playing)
        })
    }

    /// An agent's request, already checked by the core: a button, or a clean `spotify:<kind>:<id>` to play.
    private func runMediaCommand(action: String, uri: String) {
        let playing = model.earTrack?.app == "Música" ? MediaPlayer.music : MediaPlayer.spotify
        let running = MediaControl.runningPlayers()
        let player = running.contains(playing) ? playing : running.first ?? .spotify
        let script: String
        switch action {
        case "search":
            // `spotify:search:<percent-encoded>` from the core: Spotify shows the results, the user presses play.
            if let url = URL(string: uri), url.scheme == "spotify" { NSWorkspace.shared.open(url) }
            return
        case "open":
            // The core only lets through `spotify:<kind>:<22 letters/digits>`, so this string is safe to embed.
            script = "tell application id \"\(MediaPlayer.spotify.bundleID)\" to play track \"\(uri)\""
        case "play", "pause":
            script = "tell application id \"\(player.bundleID)\" to \(action)"
        case "toggle", "next", "previous":
            let command = ["toggle": MediaAction.playPause, "next": .next, "previous": .previous][action]!.command
            script = "tell application id \"\(player.bundleID)\" to \(command)"
        default:
            return
        }
        // A button never launches a closed player; playing a link may open Spotify (the user asked for it).
        guard action == "open" || running.contains(player) else { return }
        DispatchQueue.global(qos: .userInitiated).async {
            _ = MediaControl.runScript(script)
        }
    }

    // MARK: Tools

    /// Brings the terminal of a session to the front; if that app is not running (or unknown), shows its folder.
    private func open(_ place: NotchModel.Place) {
        if !place.terminal.isEmpty,
           let app = NSRunningApplication.runningApplications(withBundleIdentifier: place.terminal).first,
           app.activate() {
            return
        }
        if !place.cwd.isEmpty, FileManager.default.fileExists(atPath: place.cwd) {
            NSWorkspace.shared.open(URL(fileURLWithPath: place.cwd))
        }
    }

    private func open(_ item: Shortcut) {
        model.setHovering(false)
        if item.kind == "web" {
            if let url = ExternalLink.validated(item.target) { NSWorkspace.shared.open(url) }
        } else {
            NSWorkspace.shared.open(URL(fileURLWithPath: item.target))
        }
    }

    /// The system's open panel: apps, folders or files to pin.
    private func addShortcut() {
        let panel = NSOpenPanel()
        panel.title = "Fijar en Atajos"
        panel.prompt = "Fijar"
        panel.canChooseFiles = true
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.directoryURL = URL(fileURLWithPath: "/Applications")
        NSApp.activate()
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            model.shortcuts = try core.addShortcut(target: url.path)
        } catch {
            alert("No se pudo fijar", String(describing: error))
        }
    }

    /// The system's share menu (AirDrop, Mail, Messages…) for the dropped files.
    private func share() {
        guard let host, !model.dropped.isEmpty else { return }
        let picker = NSSharingServicePicker(items: model.dropped.filter { FileManager.default.fileExists(atPath: $0.path) })
        let size = NotchLayout.size(model, notch: geometry?.notch ?? .zero)
        let rect = NSRect(x: (host.bounds.width - size.width) / 2 + 24, y: host.bounds.height - size.height + 20, width: 120, height: 28)
        NSApp.activate()
        picker.show(relativeTo: rect, of: host, preferredEdge: .minY)
    }

    /// Keeps native hit testing and media watching in sync with SwiftUI's observed state.
    private func observe() {
        withObservationTracking {
            _ = model.mode
            _ = model.notice
            _ = model.sessions
            _ = model.nowPlaying
            _ = model.youtube
            _ = model.focus
            _ = model.shortcuts
            _ = model.dropped
            _ = model.dragging
            _ = model.buddyBusy
            _ = model.usage
            _ = model.earTrack
            _ = model.pinned
            _ = model.collapsedByUser
            _ = model.buddyActivity
            _ = model.tab
            _ = model.tools
            _ = model.status
        } onChange: { [weak self] in
            Task { @MainActor in
                guard let self else { return }
                // SwiftUI observes the model itself. Replacing its root for every session event closes open menus.
                // The players are asked only while the overview is open.
                if self.model.mode == .open { self.media.start() } else { self.media.stop() }
                if self.model.youtube?.viewer != nil && self.model.youtube?.destination == "notch" && (self.model.mode != .open || self.model.tab != .home) { self.core.youtubeClose() }
                self.syncUtilities()
                self.pointerMoved()
                self.observe()
            }
        }
    }

    private func performTool(_ work: () throws -> Void) {
        do { try work(); model.toolMessage = "" }
        catch { model.toolMessage = String(describing: error) }
    }

    private func pickShelfFiles() {
        let picker = NSOpenPanel()
        picker.title = "Añadir a la bandeja"; picker.prompt = "Añadir"
        picker.canChooseFiles = true; picker.canChooseDirectories = true; picker.allowsMultipleSelection = true
        NSApp.activate()
        picker.begin { [weak self] response in
            Task { @MainActor in
                guard response == .OK, let self else { return }
                self.performTool {
                    self.model.acceptTools(try self.core.notchAddFiles(paths: picker.urls.map(\.path)))
                    self.model.revealFiles()
                }
            }
        }
    }

    private func saveClipboard() {
        performTool {
            guard let text = NSPasteboard.general.string(forType: .string) else { throw NSError(domain: "Buddy", code: 1, userInfo: [NSLocalizedDescriptionKey: "El portapapeles no contiene texto."]) }
            model.acceptTools(try core.notchSaveClip(text: text))
        }
    }

    private func copyClip(_ clip: SavedClip) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(clip.text, forType: .string)
        model.toolMessage = "Texto copiado."
    }

    private func pickCalendar() {
        let picker = NSOpenPanel()
        picker.title = "Elegir agenda local (.ics)"; picker.prompt = "Usar agenda"
        picker.allowedContentTypes = [.init(filenameExtension: "ics")!]
        NSApp.activate()
        let core = core
        picker.begin { [weak self] response in
            Task { @MainActor in
                guard response == .OK, let url = picker.url else { return }
                let (tools, error) = await Task.detached(priority: .utility) { () -> (NotchTools?, String) in
                    do { return (try core.notchSetCalendar(path: url.path), "") }
                    catch { return (nil, String(describing: error)) }
                }.value
                guard let self else { return }
                self.model.toolMessage = error
                if let tools { self.model.acceptTools(tools); self.refreshUtilities() }
            }
        }
    }

    private func openAppointment() {
        performTool {
            if let link = model.appointment?.url, let url = ExternalLink.validated(link) { NSWorkspace.shared.open(url) }
            else { NSWorkspace.shared.open(URL(fileURLWithPath: try core.notchAppointmentFile())) }
        }
    }

    private func syncUtilities() {
        if model.mode == .open && model.tab == .files && Date().timeIntervalSince(shelfRefreshedAt) > 30 {
            shelfRefreshedAt = Date()
            if let tools = try? core.notchTools(), tools != model.tools { model.acceptTools(tools) }
        }
        let active = model.mode == .open && model.tab == .utilities
        if !active {
            utilitiesTask?.cancel(); utilitiesTask = nil
            calendarRead?.cancel(); calendarRead = nil; calendarGeneration += 1
            return
        }
        guard utilitiesTask == nil else { return }
        utilitiesTask = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                self.refreshUtilities()
                do { try await Task.sleep(for: .seconds(30)) } catch { return }
            }
        }
    }

    private func refreshUtilities() {
        model.battery = model.tools?.batteryEnabled == true ? NotchBattery.read() : nil
        calendarRead?.cancel(); calendarGeneration += 1
        guard model.tools?.calendarEnabled == true else { model.appointment = nil; return }
        let core = core; let generation = calendarGeneration
        calendarRead = Task { [weak self] in
            let (appointment, error) = await Task.detached(priority: .utility) { () -> (Appointment?, String) in
                do { return (try core.notchCalendar(), "") }
                catch { return (nil, "No se pudo leer la agenda. Vuelve a elegir el archivo .ics.") }
            }.value
            guard let self, !Task.isCancelled, generation == self.calendarGeneration else { return }
            self.model.appointment = appointment; self.model.calendarError = error
        }
    }

    private func relocate() {
        geometry = NotchGeometry.current()
        guard let geometry, let panel else { return }
        panel.setFrame(geometry.frame(for: CGSize(width: Self.canvas.width, height: geometry.notch.height + Self.canvas.height)), display: true)
        host?.rootView = view()
        relocateLockPanel()
    }

    // MARK: Hooks

    private func refreshHooks() {
        let connected = core.hooksStatus().contains { $0.installed }
        if connected != hooksConnected {
            hooksConnected = connected
            host?.rootView = view()
        }
    }

    /// Shows what would change in each agent's configuration and writes it only after "Conectar".
    private func connectHooks() {
        model.setHovering(false)
        let available = core.hooksStatus().filter { $0.available && !$0.installed }
        guard !available.isEmpty else {
            alert("No encontré Claude Code, Codex ni Gemini", "Instala uno de ellos y vuelve a intentarlo.")
            return
        }
        for status in available {
            guard let preview = try? core.hooksPreview(agent: status.agent, install: true) else { continue }
            let confirm = NSAlert()
            confirm.messageText = "¿Conectar \(status.name) con Buddy?"
            confirm.informativeText = "Buddy añadirá sus avisos (inicio y fin de sesión, permisos, notificaciones) a "
                + "\((preview.path as NSString).abbreviatingWithTildeInPath). Antes guarda una copia del archivo, "
                + "no toca tus otros hooks y puedes quitarlos cuando quieras."
            confirm.accessoryView = Self.diffView(preview.diff)
            confirm.addButton(withTitle: "Conectar")
            confirm.addButton(withTitle: "Cancelar")
            NSApp.activate()
            guard confirm.runModal() == .alertFirstButtonReturn else { continue }
            do {
                _ = try core.hooksWrite(agent: status.agent, install: true, fingerprint: preview.fingerprint)
            } catch {
                alert("No se pudo conectar \(status.name)", String(describing: error))
            }
        }
        refreshHooks()
        host?.rootView = view()
    }

    /// The exact change, in a small scrolling box (the alert itself stays short, its buttons always in view).
    private static func diffView(_ diff: String) -> NSView {
        let scroll = NSTextView.scrollableTextView()
        scroll.frame = NSRect(x: 0, y: 0, width: 460, height: 220)
        scroll.borderType = .bezelBorder
        if let text = scroll.documentView as? NSTextView {
            text.isEditable = false
            text.font = .monospacedSystemFont(ofSize: 10.5, weight: .regular)
            text.textContainerInset = NSSize(width: 6, height: 6)
            text.string = diff
        }
        return scroll
    }

    private func alert(_ title: String, _ text: String) {
        let a = NSAlert()
        a.messageText = title
        a.informativeText = text
        NSApp.activate()
        a.runModal()
    }
}

/// A panel allowed to sit over the menu bar and the notch.
final class NotchPanel: NSPanel {
    override var canBecomeKey: Bool { true }
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect { frameRect }
}
