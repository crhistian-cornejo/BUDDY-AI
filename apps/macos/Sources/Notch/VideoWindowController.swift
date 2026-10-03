import AppKit
import SwiftUI

/// Where the video sits next to Buddy. It is part of the pet: it has no frame of its own to drag, it goes where
/// Buddy goes, and when the chat opens it moves out of its way without leaving Buddy's side.
enum VideoPlacement {
    static let pictureSize = CGSize(width: 384, height: 216)
    /// Nothing around the picture: the window is the picture, with round corners and no border, haze or shadow.
    static let edge: CGFloat = 0
    static let cornerRadius: CGFloat = 16
    static let gap: CGFloat = 6
    /// How high the chat next to Buddy can get (composer, gap and the tallest chat): its whole column is kept free,
    /// so the video does not have to jump aside when an answer makes the chat grow.
    static let chatReach: CGFloat = 460
    static var windowSize: CGSize { CGSize(width: pictureSize.width + edge * 2, height: pictureSize.height + edge * 2) }

    /// The picture inside the video window's frame.
    static func picture(in frame: NSRect) -> NSRect { frame.insetBy(dx: edge, dy: edge) }

    /// The window's frame for a pet at `pet`, with the chat (composer and messages) taking `chat` when it is open,
    /// inside `area`.
    ///
    /// Without a chat: above Buddy. With one: a place by Buddy that the chat's column never reaches, and when there
    /// is none (Buddy in a corner), right on top of the chat, at its edge by Buddy, climbing as the chat grows.
    static func frame(pet: NSRect, chat: NSRect?, area: NSRect) -> NSRect {
        let (w, h) = (pictureSize.width, pictureSize.height)
        func window(_ x: CGFloat, _ y: CGFloat) -> NSRect {
            let size = windowSize
            return NSRect(x: min(max(x - edge, area.minX), max(area.minX, area.maxX - size.width)),
                          y: min(max(y - edge, area.minY), max(area.minY, area.maxY - size.height)), width: size.width, height: size.height)
        }
        func covered(_ frame: NSRect, by taken: [NSRect]) -> CGFloat {
            taken.reduce(0) { sum, rect in
                let overlap = picture(in: frame).intersection(rect)
                return sum + (overlap.isNull ? 0 : overlap.width * overlap.height)
            }
        }
        let above = window(pet.midX - w / 2, pet.maxY + gap)
        let below = window(pet.midX - w / 2, pet.minY - gap - h)
        let left = window(pet.minX - gap - w, pet.minY)
        let right = window(pet.maxX + gap, pet.minY)
        guard let chat else {
            let places = [above, below] + (area.maxX - pet.maxX >= pet.minX - area.minX ? [right, left] : [left, right])
            return places.first { covered($0, by: [pet]) == 0 } ?? places.min { covered($0, by: [pet]) < covered($1, by: [pet]) } ?? above
        }
        let chatAtLeft = chat.midX < pet.midX
        // Everything the chat may come to take: where it is now, up to its full height.
        let column = NSRect(x: chat.minX, y: chat.minY, width: chat.width, height: min(max(chat.height, chatReach), area.maxY - chat.minY))
        // Above Buddy (pushed clear of the chat's column when it is in the way), or at Buddy's other side.
        let clear = window(chatAtLeft ? max(pet.midX - w / 2, chat.maxX + gap) : min(pet.midX - w / 2, chat.minX - gap - w), pet.maxY + gap)
        let steady = [above, clear, chatAtLeft ? right : left]
        if let place = steady.first(where: { covered($0, by: [pet, column]) == 0 }) { return place }
        // On top of the chat, at its edge by Buddy.
        let onChat = window(chatAtLeft ? chat.maxX - w : chat.minX, chat.maxY + gap)
        let places = [onChat] + steady + [chatAtLeft ? left : right, below]
        return places.first { covered($0, by: [pet, chat]) == 0 } ?? places.min { covered($0, by: [pet, chat]) < covered($1, by: [pet, chat]) } ?? onChat
    }
}

/// The video next to Buddy: a window with no title, border, footer or shadow: only the picture, round at its corners.
/// Independent of the chat and the notch: closing either does not close it.
@MainActor
final class VideoWindowController {
    private let core: BuddyCore
    private let petFrame: () -> NSRect
    private let chatFrame: () -> NSRect?
    private let onAsk: () -> Void
    private var panel: NSPanel?
    private var key = ""
    private var observers: [NSObjectProtocol] = []

    init(core: BuddyCore, petFrame: @escaping () -> NSRect, chatFrame: @escaping () -> NSRect?, onAsk: @escaping () -> Void) {
        self.core = core; self.petFrame = petFrame; self.chatFrame = chatFrame; self.onAsk = onAsk
    }

    func refresh() {
        let state = core.youtubeStatus()
        guard state.destination == "floating", let video = state.viewer else { close(); return }
        let next = video.sourceId + video.videoId
        if panel != nil, next == key { return }
        let window = panel ?? Self.makePanel()
        let host = NSHostingView(rootView: VideoCompanionView(core: core, video: video, onAsk: onAsk))
        host.frame = NSRect(origin: .zero, size: VideoPlacement.windowSize)
        // Only the picture, cut to its round corners: no border, no haze, no shadow around it.
        host.wantsLayer = true
        host.layer?.mask = Self.roundMask(size: VideoPlacement.windowSize)
        window.contentView = host
        let isNew = panel == nil
        panel = window; key = next
        if isNew {
            window.setFrame(placed(), display: false)
            watch()
        }
        window.orderFrontRegardless()
    }

    private func close() {
        observers.forEach(NotificationCenter.default.removeObserver)
        observers = []
        panel?.orderOut(nil)
        panel?.contentView = nil
        panel = nil
        key = ""
    }

    private func placed() -> NSRect {
        let pet = petFrame()
        let screen = NSScreen.screens.first { $0.frame.contains(CGPoint(x: pet.midX, y: pet.midY)) } ?? NSScreen.main
        return VideoPlacement.frame(pet: pet, chat: chatFrame(), area: screen?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1280, height: 800))
    }

    /// Buddy moved (the video goes with it, at once) or the chat opened, closed or grew (the video glides aside).
    private func watch() {
        let center = NotificationCenter.default
        observers = [
            center.addObserver(forName: .petMoved, object: nil, queue: .main) { [weak self] _ in MainActor.assumeIsolated { self?.place(animated: false) } },
            center.addObserver(forName: .chatLayoutChanged, object: nil, queue: .main) { [weak self] _ in MainActor.assumeIsolated { self?.place(animated: true) } },
            center.addObserver(forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.place(animated: false) }
            },
        ]
    }

    private func place(animated: Bool) {
        guard let panel else { return }
        let target = placed()
        guard panel.frame != target else { return }
        if animated && !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            NSAnimationContext.runAnimationGroup { context in
                context.duration = 0.22
                context.timingFunction = CAMediaTimingFunction(name: .easeOut)
                panel.animator().setFrame(target, display: true)
            }
        } else {
            panel.setFrame(target, display: true)
        }
    }

    private static func makePanel() -> NSPanel {
        let panel = NSPanel(contentRect: NSRect(origin: .zero, size: VideoPlacement.windowSize),
                            styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        panel.isReleasedWhenClosed = false
        panel.hidesOnDeactivate = false
        // It moves with Buddy, never by itself.
        panel.isMovable = false
        // No frame to read it from: VoiceOver gets the window's name here.
        panel.title = "Video junto a Buddy"
        panel.setAccessibilityLabel("Video junto a Buddy")
        return panel
    }

    /// The picture's shape: a rectangle with round corners.
    static func roundMask(size: CGSize) -> CALayer {
        let mask = CAShapeLayer()
        mask.frame = CGRect(origin: .zero, size: size)
        let picture = VideoPlacement.picture(in: mask.frame)
        mask.path = CGPath(roundedRect: picture, cornerWidth: VideoPlacement.cornerRadius, cornerHeight: VideoPlacement.cornerRadius, transform: nil)
        return mask
    }
}

/// The player filling the window, with Buddy's few actions over it while the pointer is on the video (and for a
/// moment when it appears, so they are found). They stay reachable for VoiceOver and the keyboard at all times.
private struct VideoCompanionView: View {
    let core: BuddyCore
    let video: YouTubeVideo
    let onAsk: () -> Void
    @State private var error = ""
    @State private var hovering = false
    @State private var introduced = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        ZStack {
            Color.black
            YouTubeWebPlayer(video: video) { type, code in
                if type == "playing" { core.youtubeStarted(sourceId: video.sourceId, videoId: video.videoId); error = "" }
                if type == "position" { core.youtubePosition(sourceId: video.sourceId, videoId: video.videoId, seconds: Double(code)) }
                if type == "error" { error = "Este video no permite reproducción aquí. Ábrelo en YouTube." }
                if type == "blocked" { error = "Pulsa reproducir en el video." }
            }
            .accessibilityLabel("Video: \(video.title)")
            VStack(spacing: 0) {
                HStack(spacing: 2) {
                    action("sparkles", "Preguntar a Gemini sobre este video", onAsk)
                    action("rectangle.topthird.inset.filled", "Pasar el video al notch") { try? core.youtubeMove(destination: "notch") }
                    action("arrow.up.right", "Abrir en YouTube") { if let url = URL(string: video.url) { NSWorkspace.shared.open(url) } }
                    action("xmark", "Cerrar el video") { core.youtubeClose() }
                }
                .padding(3)
                .background(.black.opacity(0.7), in: Capsule())
                .fixedSize()
                .frame(maxWidth: .infinity, alignment: .trailing)
                .opacity(hovering || !introduced || !error.isEmpty ? 1 : 0)
                .accessibilityElement(children: .contain)
                .accessibilityLabel("Acciones del video")
                Spacer()
                if !error.isEmpty {
                    Text(error)
                        .font(.callout)
                        .foregroundStyle(.white)
                        .padding(.horizontal, 12).padding(.vertical, 6)
                        .background(.black.opacity(0.75), in: Capsule())
                        .padding(.bottom, 46)
                }
            }
            .padding(VideoPlacement.edge + 8)
        }
        .frame(width: VideoPlacement.windowSize.width, height: VideoPlacement.windowSize.height)
        .onHover { inside in withAnimation(reduceMotion ? nil : .easeOut(duration: 0.15)) { hovering = inside } }
        .task {
            // Shown for a moment when the video appears, then only under the pointer.
            try? await Task.sleep(for: .seconds(4))
            withAnimation(reduceMotion ? nil : .easeOut(duration: 0.3)) { introduced = true }
        }
    }

    /// A round button of at least 28 points, named for VoiceOver and with its name as help.
    private func action(_ symbol: String, _ help: String, _ run: @escaping () -> Void) -> some View {
        Button(action: run) {
            Image(systemName: symbol)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: 30, height: 28)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(help)
        .accessibilityLabel(help)
    }
}

extension Notification.Name {
    static let buddyVideoQuestion = Notification.Name("buddyVideoQuestion")
    /// The chat next to Buddy opened, closed, moved or changed size.
    static let chatLayoutChanged = Notification.Name("buddy.chatLayoutChanged")
}
