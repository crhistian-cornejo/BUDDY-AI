import AppKit
import SwiftUI

/// Where the video sits next to Buddy. It is part of the pet: it has no frame of its own to drag, it goes where
/// Buddy goes, and when the chat opens it moves out of its way without leaving Buddy's side.
enum VideoPlacement {
    static let pictureSize = CGSize(width: 384, height: 216)
    /// The soft edge around the picture, inside the window: the picture fades out over it instead of ending in a border.
    static let edge: CGFloat = 18
    static let gap: CGFloat = 6
    static var windowSize: CGSize { CGSize(width: pictureSize.width + edge * 2, height: pictureSize.height + edge * 2) }

    /// The picture inside the video window's frame.
    static func picture(in frame: NSRect) -> NSRect { frame.insetBy(dx: edge, dy: edge) }

    /// The window's frame for a pet at `pet`, with the chat (composer and messages) taking `chat` when it is open,
    /// inside `area`. Above Buddy when that is free; otherwise the first place by Buddy that covers neither Buddy
    /// nor the chat.
    static func frame(pet: NSRect, chat: NSRect?, area: NSRect) -> NSRect {
        let (w, h) = (pictureSize.width, pictureSize.height)
        func window(_ x: CGFloat, _ y: CGFloat) -> NSRect {
            let size = windowSize
            return NSRect(x: min(max(x - edge, area.minX), max(area.minX, area.maxX - size.width)),
                          y: min(max(y - edge, area.minY), max(area.minY, area.maxY - size.height)), width: size.width, height: size.height)
        }
        let above = window(pet.midX - w / 2, pet.maxY + gap)
        let below = window(pet.midX - w / 2, pet.minY - gap - h)
        let left = window(pet.minX - gap - w, pet.minY)
        let right = window(pet.maxX + gap, pet.minY)
        var places: [NSRect]
        if let chat {
            let chatAtLeft = chat.midX < pet.midX
            // Above Buddy, pushed clear of the chat's column; then Buddy's other side; then on top of the chat.
            let clear = window(chatAtLeft ? max(pet.midX - w / 2, chat.maxX + gap) : min(pet.midX - w / 2, chat.minX - gap - w), pet.maxY + gap)
            let overChat = window(chatAtLeft ? chat.maxX - w : chat.minX, chat.maxY + gap)
            places = [above, clear, chatAtLeft ? right : left, overChat, chatAtLeft ? left : right, below]
        } else {
            places = [above, below] + (area.maxX - pet.maxX >= pet.minX - area.minX ? [right, left] : [left, right])
        }
        let taken = [pet] + (chat.map { [$0] } ?? [])
        func covered(_ frame: NSRect) -> CGFloat {
            taken.reduce(0) { sum, rect in
                let overlap = picture(in: frame).intersection(rect)
                return sum + (overlap.isNull ? 0 : overlap.width * overlap.height)
            }
        }
        return places.first { covered($0) == 0 } ?? places.min { covered($0) < covered($1) } ?? above
    }
}

/// The video next to Buddy: a window with no title, border or footer, only the picture with its edge fading out.
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
        // The whole content fades out towards the window's edge: no border, no shadow, a soft cloud.
        host.wantsLayer = true
        host.layer?.mask = Self.softMask(size: VideoPlacement.windowSize, scale: window.backingScaleFactor)
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
        return panel
    }

    /// A mask that is solid over the picture and fades to nothing over `VideoPlacement.edge`: a rounded rectangle
    /// drawn with a blurred shadow of itself.
    static func softMask(size: CGSize, scale: CGFloat) -> CALayer {
        let mask = CALayer()
        mask.frame = CGRect(origin: .zero, size: size)
        mask.contentsScale = scale
        let (width, height) = (Int(size.width * scale), Int(size.height * scale))
        guard let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(),
                                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else {
            mask.backgroundColor = NSColor.black.cgColor
            return mask
        }
        context.scaleBy(x: scale, y: scale)
        let edge = VideoPlacement.edge
        let solid = CGRect(origin: .zero, size: size).insetBy(dx: edge, dy: edge)
        context.setShadow(offset: .zero, blur: edge * 0.9, color: NSColor.black.cgColor)
        context.setFillColor(NSColor.black.cgColor)
        context.addPath(CGPath(roundedRect: solid, cornerWidth: 20, cornerHeight: 20, transform: nil))
        context.fillPath()
        mask.contents = context.makeImage()
        return mask
    }
}

/// The player filling the window (its outer band is what fades out), with Buddy's few actions over it only while
/// the pointer is on the video.
private struct VideoCompanionView: View {
    let core: BuddyCore
    let video: YouTubeVideo
    let onAsk: () -> Void
    @State private var error = ""
    @State private var hovering = false

    var body: some View {
        ZStack {
            // Behind the player and under the soft edge: the fade ends in a dark haze, not in the desktop's colours.
            Color.black
            YouTubeWebPlayer(video: video) { type, code in
                if type == "playing" { core.youtubeStarted(sourceId: video.sourceId, videoId: video.videoId); error = "" }
                if type == "position" { core.youtubePosition(sourceId: video.sourceId, videoId: video.videoId, seconds: Double(code)) }
                if type == "error" { error = "Este video no permite reproducción aquí. Ábrelo en YouTube." }
                if type == "blocked" { error = "Pulsa reproducir en el video." }
            }
            .padding(VideoPlacement.edge - 6)
            VStack(spacing: 0) {
                HStack(spacing: 2) {
                    action("sparkles", "Preguntar a Gemini sobre este video", onAsk)
                    action("rectangle.topthird.inset.filled", "Pasar al notch") { try? core.youtubeMove(destination: "notch") }
                    action("arrow.up.right", "Abrir en YouTube") { if let url = URL(string: video.url) { NSWorkspace.shared.open(url) } }
                    action("xmark", "Cerrar el video") { core.youtubeClose() }
                }
                .padding(4)
                .background(.black.opacity(0.55), in: Capsule())
                .fixedSize()
                .frame(maxWidth: .infinity, alignment: .trailing)
                .opacity(hovering ? 1 : 0)
                Spacer()
                if !error.isEmpty {
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.white)
                        .padding(.horizontal, 10).padding(.vertical, 5)
                        .background(.black.opacity(0.6), in: Capsule())
                        .padding(.bottom, 46)
                }
            }
            .padding(VideoPlacement.edge + 6)
        }
        .frame(width: VideoPlacement.windowSize.width, height: VideoPlacement.windowSize.height)
        .onHover { inside in withAnimation(.easeOut(duration: 0.15)) { hovering = inside } }
    }

    private func action(_ symbol: String, _ help: String, _ run: @escaping () -> Void) -> some View {
        Button(action: run) {
            Image(systemName: symbol)
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: 24, height: 22)
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
