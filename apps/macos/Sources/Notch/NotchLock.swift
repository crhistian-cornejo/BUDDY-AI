import AppKit
import Observation
import SwiftUI

/// Separate from the regular island: no session, clipboard or notification content reaches loginwindow.
@MainActor
@Observable
final class NotchLockState {
    enum Phase { case hidden, locked, unlocking }
    private(set) var phase: Phase = .hidden
    @ObservationIgnored private var dismissal: Task<Void, Never>?
    @ObservationIgnored var onFinish: (() -> Void)?

    func lock() {
        dismissal?.cancel()
        phase = .locked
    }

    func unlock(seconds: Double = 0.65) {
        guard phase == .locked else { return }
        phase = .unlocking
        dismissal = Task { [weak self] in
            do { try await Task.sleep(for: .seconds(seconds)) } catch { return }
            guard let self, !Task.isCancelled, self.phase == .unlocking else { return }
            self.phase = .hidden
            self.onFinish?()
        }
    }

    func stop() {
        dismissal?.cancel()
        dismissal = nil
        phase = .hidden
    }
}

struct NotchLockView: View {
    let state: NotchLockState
    let notch: CGSize
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var opened: Bool { state.phase == .unlocking }

    var body: some View {
        HStack(spacing: 0) {
            NotchLockGlyph(opened: opened)
                .scaleEffect(0.8)
                .frame(width: 48)
            Color.clear.frame(width: notch.width, height: notch.height)
            Image(systemName: "checkmark")
                .opacity(opened ? 1 : 0)
                .font(.system(size: 11, weight: .semibold))
                .frame(width: 48)
        }
        .foregroundStyle(.white)
        .frame(height: notch.height)
        .background(NotchShape(earRadius: 6, bottomRadius: 10).fill(.black))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(opened ? "Mac desbloqueado" : "Mac bloqueado")
        .animation(reduceMotion ? nil : .easeOut(duration: 0.18), value: opened)
        .environment(\.colorScheme, .dark)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .ignoresSafeArea()
    }
}

/// Animate the shackle itself, without the delayed fade/replacement of an SF Symbol.
private struct NotchLockGlyph: View {
    let opened: Bool

    var body: some View {
        ZStack(alignment: .top) {
            NotchLockShackle()
                .stroke(style: StrokeStyle(lineWidth: 2.3, lineCap: .round))
                .frame(width: 11, height: 13)
                .rotationEffect(.degrees(opened ? 35 : 0), anchor: .bottomTrailing)
                .offset(y: opened ? -2 : 0)
            RoundedRectangle(cornerRadius: 3)
                .frame(width: 17, height: 12)
                .offset(y: 10)
        }
        .frame(width: 25, height: 24)
    }
}

private struct NotchLockShackle: Shape {
    func path(in rect: CGRect) -> Path {
        let radius = rect.width / 2
        var path = Path()
        path.move(to: CGPoint(x: rect.minX, y: rect.maxY))
        path.addLine(to: CGPoint(x: rect.minX, y: rect.minY + radius))
        path.addArc(center: CGPoint(x: rect.midX, y: rect.minY + radius), radius: radius,
                    startAngle: .degrees(180), endAngle: .degrees(0), clockwise: false)
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.maxY))
        return path
    }
}

/// Sample the session flag only while locked. Distributed unlock notifications can arrive after
/// loginwindow has already started returning to the desktop; the flag avoids that delivery delay.
@MainActor
final class NotchUnlockMonitor {
    private var timer: Timer?
    private let readLocked: @MainActor () -> Bool?
    var onUnlock: (() -> Void)?

    init(readLocked: @escaping @MainActor () -> Bool? = NotchUnlockMonitor.sessionLocked) {
        self.readLocked = readLocked
    }

    static func sessionLocked() -> Bool? {
        guard let session = CGSessionCopyCurrentDictionary() as? [String: Any],
              session[kCGSessionOnConsoleKey as String] as? Bool == true else { return nil }
        return (session["CGSSessionScreenIsLocked"] as? Bool) ?? false
    }

    func start() {
        guard timer == nil else { return }
        let timer = Timer(timeInterval: 1.0 / 60, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.check() }
        }
        timer.tolerance = 0.002
        self.timer = timer
        RunLoop.main.add(timer, forMode: .common)
        check()
    }

    func check() {
        guard timer != nil, readLocked() == false else { return }
        stop()
        onUnlock?()
    }

    func stop() {
        timer?.invalidate()
        timer = nil
    }
}

final class NotchLockPanel: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect { frameRect }
}

/// Loginwindow uses a separate space. A high window level alone cannot put an ordinary panel there.
/// Dynamic lookup keeps this optional if macOS removes these private entry points.
/// API signatures/reference: https://github.com/Lakr233/SkyLightWindow (MIT).
final class NotchLockSpace {
    private typealias Connection = @convention(c) () -> Int32
    private typealias Create = @convention(c) (Int32, Int32, Int32) -> Int32
    private typealias Level = @convention(c) (Int32, Int32, Int32) -> Int32
    private typealias Show = @convention(c) (Int32, CFArray) -> Void
    private typealias Add = @convention(c) (Int32, Int32, CFArray, Int32) -> Void
    private typealias Destroy = @convention(c) (Int32, Int32) -> Void
    private let handle: UnsafeMutableRawPointer
    private let connection: Int32
    private let space: Int32
    private let add: Add
    private let destroy: Destroy

    init?() {
        guard let handle = dlopen("/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight", RTLD_LAZY) else { return nil }
        guard let connectionSymbol = dlsym(handle, "SLSMainConnectionID"),
              let createSymbol = dlsym(handle, "SLSSpaceCreate"),
              let levelSymbol = dlsym(handle, "SLSSpaceSetAbsoluteLevel"),
              let showSymbol = dlsym(handle, "SLSShowSpaces"),
              let addSymbol = dlsym(handle, "SLSSpaceAddWindowsAndRemoveFromSpaces"),
              let destroySymbol = dlsym(handle, "SLSSpaceDestroy") else {
            dlclose(handle)
            return nil
        }
        let connection = unsafeBitCast(connectionSymbol, to: Connection.self)()
        let space = unsafeBitCast(createSymbol, to: Create.self)(connection, 1, 0)
        let destroy = unsafeBitCast(destroySymbol, to: Destroy.self)
        guard space != 0,
              unsafeBitCast(levelSymbol, to: Level.self)(connection, space, 400) == 0 else {
            if space != 0 { destroy(connection, space) }
            dlclose(handle)
            return nil
        }
        unsafeBitCast(showSymbol, to: Show.self)(connection, [space] as CFArray)
        self.handle = handle
        self.connection = connection
        self.space = space
        self.add = unsafeBitCast(addSymbol, to: Add.self)
        self.destroy = destroy
    }

    @MainActor
    func attach(_ panel: NSPanel) {
        add(connection, space, [panel.windowNumber] as CFArray, 7)
    }

    deinit {
        destroy(connection, space)
        dlclose(handle)
    }
}
