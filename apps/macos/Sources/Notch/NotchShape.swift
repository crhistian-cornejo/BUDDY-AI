// The island's outline: concave "ears" at the top (it hangs from the menu bar like the hardware notch) and rounded
// bottom corners. Animatable, so it can grow from the notch's own size. Geometry ported from MIKA's IslandRootView
// (MIT, revision d050bc5).
import SwiftUI

struct NotchShape: Shape {
    var earRadius: CGFloat
    var bottomRadius: CGFloat

    var animatableData: AnimatablePair<CGFloat, CGFloat> {
        get { .init(earRadius, bottomRadius) }
        set { earRadius = newValue.first; bottomRadius = newValue.second }
    }

    func path(in rect: CGRect) -> Path {
        let (w, h) = (rect.width, rect.height)
        let er = max(0, min(earRadius, w / 4, h / 2))
        let br = max(0, min(bottomRadius, (w - 2 * er) / 2, h - er))
        var p = Path()
        p.move(to: CGPoint(x: 0, y: 0))
        p.addArc(center: CGPoint(x: 0, y: er), radius: er, startAngle: .degrees(270), endAngle: .degrees(0), clockwise: false)
        p.addLine(to: CGPoint(x: er, y: h - br))
        p.addArc(center: CGPoint(x: er + br, y: h - br), radius: br, startAngle: .degrees(180), endAngle: .degrees(90), clockwise: true)
        p.addLine(to: CGPoint(x: w - er - br, y: h))
        p.addArc(center: CGPoint(x: w - er - br, y: h - br), radius: br, startAngle: .degrees(90), endAngle: .degrees(0), clockwise: true)
        p.addLine(to: CGPoint(x: w - er, y: er))
        p.addArc(center: CGPoint(x: w, y: er), radius: er, startAngle: .degrees(180), endAngle: .degrees(270), clockwise: false)
        p.closeSubpath()
        return p
    }
}

/// Where the island lives: the screen with a notch (its exact size), or the main screen with a small pill.
@MainActor
struct NotchGeometry {
    let screen: NSScreen
    /// The hardware notch, or a pill-sized stand-in on screens without one.
    let notch: CGSize
    let hasNotch: Bool

    static func current() -> NotchGeometry? {
        if let screen = NSScreen.screens.first(where: { $0.safeAreaInsets.top > 0 }) {
            let aux = (screen.auxiliaryTopLeftArea?.width ?? 0) + (screen.auxiliaryTopRightArea?.width ?? 0)
            let width = screen.frame.width - aux
            return NotchGeometry(screen: screen, notch: CGSize(width: width > 0 ? width : 190, height: screen.safeAreaInsets.top),
                                 hasNotch: true)
        }
        guard let screen = NSScreen.main else { return nil }
        let menuBar = screen.frame.maxY - screen.visibleFrame.maxY
        return NotchGeometry(screen: screen, notch: CGSize(width: 190, height: max(menuBar, 24)), hasNotch: false)
    }

    /// The window frame for an island of `size`, centred under the top edge.
    func frame(for size: CGSize) -> NSRect {
        NSRect(x: screen.frame.midX - size.width / 2, y: screen.frame.maxY - size.height, width: size.width, height: size.height)
    }
}
