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
    static let canvas = CGSize(width: 640, height: 320)

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
        model.shortcuts = (try? core.shortcuts()) ?? []
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
                             title: "\(name) pide permiso en \(project)", detail: "\(title): \(summary)", command: detail))
            NSSound(named: "Tink")?.play()
        case let .approvalClosed(requestId):
            approvalSessions[requestId] = nil
            model.closeApproval(requestId)
        case let .focusChanged(running, _):
            model.focus = running ? core.focusStatus() : nil
        case let .focusFinished(minutes):
            model.focus = nil
            model.show(.init(kind: .finished, agent: "buddy", title: "Terminó tu bloque de enfoque",
                             detail: "\(minutes) minutos. Tómate un respiro."))
            NSSound(named: "Glass")?.play()
        case let .sessionUpdate(sessionId, agent, project, state):
            let previous = model.sessions.first { $0.id == sessionId }?.state
            model.update(session: sessionId, agent: agent, project: project, state: state)
            guard previous != state else { break }
            let name = agent == "codex" ? "Codex" : "Claude Code"
            switch state {
            case "done": model.show(.init(kind: .finished, agent: agent, title: "\(name) terminó", detail: project))
            // A permission request also puts the session in "waiting": its own card says it better.
            case "waiting":
                // The core sends the request a moment after this state: wait for it before saying anything.
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { [weak self] in
                    guard let self, !self.hasPendingApproval(for: sessionId),
                          self.model.sessions.first(where: { $0.id == sessionId })?.state == "waiting" else { return }
                    self.model.show(.init(kind: .waiting, agent: agent, title: "\(name) espera tu respuesta", detail: project))
                }
            case "error": model.show(.init(kind: .failed, agent: agent, title: "\(name) se detuvo por un error", detail: project))
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
        let trigger = model.mode == .idle ? geometry.frame(for: geometry.notch).insetBy(dx: -6, dy: -2) : islandRect
        let inside = trigger.contains(p)
        panel.ignoresMouseEvents = !inside
        if inside {
            leaveWork?.cancel(); leaveWork = nil
            guard !model.hovering, hoverWork == nil else { return }
            let work = DispatchWorkItem { [weak self] in
                guard let self else { return }
                self.hoverWork = nil
                self.refreshHooks()
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
                      connect: { [weak self] in self?.connectHooks() },
                      media: { [weak self] action in self?.media.send(action) },
                      focusStart: { [weak self] minutes in _ = self?.core.focusStart(minutes: minutes) },
                      focusStop: { [weak self] in self?.core.focusStop() },
                      openShortcut: { [weak self] item in self?.open(item) },
                      addShortcut: { [weak self] in self?.addShortcut() },
                      removeShortcut: { [weak self] item in
                          self?.model.shortcuts = (try? self?.core.removeShortcut(id: item.id)) ?? self?.model.shortcuts ?? []
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

    // MARK: Tools

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
            confirm.informativeText = "Buddy añadirá sus avisos a \(preview.path) y guardará antes una copia. "
                + "No toca nada más de tu configuración; puedes quitarlos cuando quieras.\n\n"
                + String(preview.diff.prefix(1200))
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
