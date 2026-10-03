import AppKit
import SwiftUI

/// The island in the notch (or a pill at the top of a screen without one). The window is a fixed transparent area at
/// the top centre; it only takes the mouse inside the island's current shape, so everything around it stays clickable.
/// Nothing runs while the pointer is still and nothing is shown.
@MainActor
final class NotchController {
    private let core: BuddyCore
    private let model = NotchModel()
    private var geometry: NotchGeometry?
    private var panel: NotchPanel?
    private var host: NSHostingView<NotchView>?
    private var monitors: [Any] = []
    private var hoverWork: DispatchWorkItem?
    private var leaveWork: DispatchWorkItem?
    private let media = MediaWatcher()
    private var hooksConnected = false

    /// Opens a chat by id next to Buddy.
    var onOpenChat: ((String) -> Void)?
    /// Files handed to Buddy from the drop zone (opens the chat with them).
    var onGiveFiles: (([URL]) -> Void)?

    /// The most the island ever needs; the window keeps this size.
    static let canvas = CGSize(width: 640, height: 460)

    init(core: BuddyCore) {
        self.core = core
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
        media.onChange = { [weak self] track in
            self?.model.nowPlaying = track
            self?.model.nowPlayingAt = Date()
        }
        model.focus = core.focusStatus().running ? core.focusStatus() : nil
        watchPlayers()
        model.shortcuts = (try? core.shortcuts()) ?? []
        model.usage = core.usage()
        model.briefing = core.briefing()
        observe()
        let handler: (NSEvent) -> Void = { [weak self] _ in MainActor.assumeIsolated { self?.pointerMoved() } }
        monitors.append(NSEvent.addGlobalMonitorForEvents(matching: [.mouseMoved, .leftMouseDragged], handler: handler) as Any)
        monitors.append(NSEvent.addLocalMonitorForEvents(matching: [.mouseMoved, .leftMouseDragged]) { event in
            handler(event)
            return event
        } as Any)
        #if DEBUG
        // BUDDY_DEBUG_NOTCH=open|drop: shows that state at launch, to look at it without touching the mouse.
        switch ProcessInfo.processInfo.environment["BUDDY_DEBUG_NOTCH"] {
        case "open": DispatchQueue.main.asyncAfter(deadline: .now() + 1) { self.debugPinned = true; self.model.setHovering(true) }
        case "drop": DispatchQueue.main.asyncAfter(deadline: .now() + 1) { self.model.dropped = [URL(fileURLWithPath: NSHomeDirectory() + "/Downloads")] }
        default: break
        }
        #endif
        NotificationCenter.default.addObserver(forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.relocate() }
        }
    }

    // MARK: Core events

    func handle(_ event: Event) {
        switch event {
        case let .approvalRequest(requestId, sessionId, agent, project, title, summary, detail, canAllow):
            approvalSessions[requestId] = sessionId
            let name = agent == "codex" ? "Codex" : "Claude Code"
            model.show(.init(kind: .approval(requestID: requestId, canAllow: canAllow), agent: agent,
                             title: agent == "buddy" ? "Buddy quiere \(title.prefix(1).lowercased() + title.dropFirst())" : "\(name) pide permiso en \(project)",
                             detail: "\(title): \(summary)", command: detail))
            NSSound(named: "Tink")?.play()
        case let .approvalClosed(requestId):
            approvalSessions[requestId] = nil
            model.closeApproval(requestId)
        case .usageChanged:
            model.usage = core.usage()
        case .briefingReady:
            model.briefing = core.briefing()
        case let .mediaCommand(action, uri):
            runMediaCommand(action: action, uri: uri)
        case let .usageLow(provider, label, leftPct):
            let name = provider == "codex" ? "Codex" : "Claude"
            model.show(.init(kind: .waiting, agent: provider, title: "Te queda \(leftPct) % de \(name)",
                             detail: "Ventana: \(label). Buddy usará el otro proveedor si se acaba."))
        case let .mascotState(state):
            model.buddyBusy = state == "think" || state == "work"
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
            let name = agent == "codex" ? "Codex" : "Claude Code"
            let place = cwd.isEmpty && terminal.isEmpty ? nil : NotchModel.Place(cwd: cwd, terminal: terminal)
            switch state {
            case "done":
                // What it said last when the agent tells us (Claude Code and Codex send it on Stop), else the project.
                model.show(.init(kind: .finished, agent: agent, title: "\(name) terminó en \(project)",
                                 detail: summary.isEmpty ? "Toca para volver a la sesión." : summary, place: place))
            // A permission request also puts the session in "waiting": its own card says it better.
            case "waiting":
                // The core sends the request a moment after this state: wait for it before saying anything.
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { [weak self] in
                    guard let self, !self.hasPendingApproval(for: sessionId),
                          self.model.sessions.first(where: { $0.id == sessionId })?.state == "waiting" else { return }
                    self.model.show(.init(kind: .waiting, agent: agent, title: "\(name) espera tu respuesta", detail: project, place: place))
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
        guard let panel, let geometry else { return }
        let p = NSEvent.mouseLocation
        // While idle, the notch itself (a little wider) is what opens the island.
        // The pointer at the very top of the screen sits exactly on the rectangle's upper edge, which `contains` leaves
        // out: it flickered in and out there, opening and closing the island. The areas reach past the top edge, and
        // once open the island gets a margin so the pointer hugging its border does not close it.
        let trigger = model.mode == .idle ? geometry.frame(for: geometry.notch).insetBy(dx: -6, dy: -2) : islandRect.insetBy(dx: -10, dy: -10)
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

    // MARK: Drawing

    private func view() -> NotchView {
        NotchView(model: model, notch: geometry?.notch ?? CGSize(width: 190, height: 32), hooksConnected: hooksConnected,
                  actions: NotchActions(
                      answer: { [weak self] id, allow in
                          self?.core.answerApproval(requestId: id, allow: allow)
                          self?.model.closeApproval(id)
                      },
                      openPlace: { [weak self] place in self?.open(place) },
                      connect: { [weak self] in self?.connectHooks() },
                      media: { [weak self] action in self?.media.send(action) },
                      seek: { [weak self] ms in self?.media.seek(toMs: ms) },
                      focusStart: { [weak self] minutes in _ = self?.core.focusStart(minutes: minutes) },
                      focusStop: { [weak self] in self?.core.focusStop() },
                      openShortcut: { [weak self] item in self?.open(item) },
                      addShortcut: { [weak self] in self?.addShortcut() },
                      removeShortcut: { [weak self] item in
                          self?.model.shortcuts = (try? self?.core.removeShortcut(id: item.id)) ?? self?.model.shortcuts ?? []
                      },
                      openLink: { link in
                          // Only web links from the briefing; anything else is ignored.
                          guard let url = URL(string: link), ["https", "http"].contains(url.scheme?.lowercased() ?? "") else { return }
                          NSWorkspace.shared.open(url)
                      },
                      giveToBuddy: { [weak self] in
                          guard let self else { return }
                          let files = self.model.dropped
                          self.model.dropped = []
                          self.onGiveFiles?(files)
                      },
                      share: { [weak self] in self?.share() },
                      copyPaths: { [weak self] in
                          guard let self else { return }
                          NSPasteboard.general.clearContents()
                          NSPasteboard.general.setString(self.model.dropped.map(\.path).joined(separator: "\n"), forType: .string)
                          self.model.dropped = []
                      },
                      clearDrop: { [weak self] in self?.model.dropped = [] },
                      drop: { [weak self] urls in
                          self?.model.dragging = false
                          if !urls.isEmpty { self?.model.dropped = urls }
                      }))
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
        let picker = NSSharingServicePicker(items: model.dropped)
        let size = NotchLayout.size(model, notch: geometry?.notch ?? .zero)
        let rect = NSRect(x: (host.bounds.width - size.width) / 2 + 24, y: host.bounds.height - size.height + 20, width: 120, height: 28)
        NSApp.activate()
        picker.show(relativeTo: rect, of: host, preferredEdge: .minY)
    }

    /// Redraws with fresh inputs whenever the model changes.
    private func observe() {
        withObservationTracking {
            _ = model.mode
            _ = model.notice
            _ = model.sessions
            _ = model.nowPlaying
            _ = model.focus
            _ = model.shortcuts
            _ = model.dropped
            _ = model.dragging
            _ = model.buddyBusy
            _ = model.usage
            _ = model.earTrack
        } onChange: { [weak self] in
            Task { @MainActor in
                guard let self else { return }
                self.host?.rootView = self.view()
                // The players are asked only while the overview is open.
                if self.model.mode == .open { self.media.start() } else { self.media.stop() }
                self.pointerMoved()
                self.observe()
            }
        }
    }

    private func relocate() {
        geometry = NotchGeometry.current()
        guard let geometry, let panel else { return }
        panel.setFrame(geometry.frame(for: CGSize(width: Self.canvas.width, height: geometry.notch.height + Self.canvas.height)), display: true)
        host?.rootView = view()
    }

    // MARK: Hooks

    private func refreshHooks() {
        hooksConnected = core.hooksStatus().contains { $0.installed }
    }

    /// Shows what would change in each agent's configuration and writes it only after "Conectar".
    private func connectHooks() {
        model.setHovering(false)
        let available = core.hooksStatus().filter { $0.available && !$0.installed }
        guard !available.isEmpty else {
            alert("No encontré Claude Code ni Codex", "Instala uno de los dos y vuelve a intentarlo.")
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
