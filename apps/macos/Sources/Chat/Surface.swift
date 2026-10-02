import AppKit
import SwiftUI

/// Buddy's floating surfaces (composer, chat, history, hello bubble). Like MIKA's island: the window is transparent and
/// has no system shadow; the glass, the rounded shape, a hairline and a soft shadow are drawn inside it, with a
/// transparent margin around so the shadow is never clipped into a box.
enum Surface {
    /// Transparent space around the shape inside its window (room for the shadow).
    static let margin: CGFloat = 18

    /// The window frame for a surface whose visible rectangle is `visible`.
    static func windowFrame(for visible: NSRect) -> NSRect { visible.insetBy(dx: -margin, dy: -margin) }

    /// The visible rectangle of a surface window.
    static func visibleFrame(of window: NSWindow) -> NSRect { window.frame.insetBy(dx: margin, dy: margin) }
}

extension View {
    /// The view on Buddy's surface: system glass, continuous rounded corners, hairline, soft shadow and the margin.
    func buddySurface(cornerRadius: CGFloat) -> some View {
        let shape = RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
        return self
            .background(GlassBackdrop(cornerRadius: cornerRadius))
            .clipShape(shape)
            // Tooltips are drawn by the surface itself: the system's do not show in these panels.
            .tooltipHost()
            .overlay(shape.strokeBorder(Color.primary.opacity(0.1), lineWidth: 0.5))
            .shadow(color: .black.opacity(0.18), radius: 12, y: 4)
            .padding(Surface.margin)
    }
}

/// The system's glass behind the content: Liquid Glass (NSGlassEffectView) on macOS 26 and later, the popover
/// material blending with what is behind the window before. Both follow light and dark mode.
struct GlassBackdrop: NSViewRepresentable {
    let cornerRadius: CGFloat

    func makeNSView(context: Context) -> NSView {
        if #available(macOS 26.0, *) {
            let glass = NSGlassEffectView()
            glass.cornerRadius = cornerRadius
            return glass
        }
        let effect = NSVisualEffectView()
        effect.material = .popover
        effect.blendingMode = .behindWindow
        effect.state = .active
        return effect
    }

    func updateNSView(_ view: NSView, context: Context) {
        if #available(macOS 26.0, *), let glass = view as? NSGlassEffectView {
            glass.cornerRadius = cornerRadius
        }
    }
}

/// A borderless, transparent panel that can take the keyboard without a title bar (composer, chat, history).
final class KeyPanel: NSPanel {
    override var canBecomeKey: Bool { true }

    /// A panel showing `view` on Buddy's surface; `visibleSize` is the shape's size (the window adds the margin).
    static func make<V: View>(visibleSize: CGSize, cornerRadius: CGFloat, view: V) -> (KeyPanel, NSHostingView<AnyView>) {
        let frame = Surface.windowFrame(for: NSRect(origin: .zero, size: visibleSize))
        let panel = KeyPanel(contentRect: frame, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        panel.isReleasedWhenClosed = false
        panel.hidesOnDeactivate = false
        panel.becomesKeyOnlyIfNeeded = false
        let host = NSHostingView(rootView: AnyView(view.buddySurface(cornerRadius: cornerRadius)))
        // The window decides the size; SwiftUI fills it.
        host.sizingOptions = []
        panel.contentView = host
        return (panel, host)
    }
}
