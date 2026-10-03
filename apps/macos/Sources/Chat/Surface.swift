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
    func buddySurface(cornerRadius: CGFloat, prominent: Bool = false) -> some View {
        let shape = RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
        return self
            .background {
                if prominent {
                    Color.dynamic(light: "#FFFFFF", dark: "#29292D")
                } else {
                    GlassBackdrop(cornerRadius: cornerRadius)
                }
            }
            .clipShape(shape)
            // Tooltips are drawn by the surface itself: the system's do not show in these panels.
            .tooltipHost()
            // Barely there: a hairline that only separates it from a background of the same colour, a very soft shadow.
            .overlay(shape.strokeBorder(Color.dynamic(light: "#000000", dark: "#FFFFFF").opacity(prominent ? 0.24 : 0.05), lineWidth: prominent ? 1 : 0.5))
            .shadow(color: .black.opacity(prominent ? 0.4 : 0.08), radius: prominent ? 14 : 10, y: prominent ? 5 : 3)
            .padding(Surface.margin)
    }
}

/// The system's popover material behind the content: blurs what is behind the window and follows light and dark
/// mode, without the bright rim Liquid Glass draws on its edge (which read as a grey border here).
struct GlassBackdrop: NSViewRepresentable {
    let cornerRadius: CGFloat

    func makeNSView(context: Context) -> NSVisualEffectView {
        let effect = NSVisualEffectView()
        effect.material = .popover
        effect.blendingMode = .behindWindow
        effect.state = .active
        return effect
    }

    func updateNSView(_ view: NSVisualEffectView, context: Context) {}
}

/// A borderless, transparent panel that can take the keyboard without a title bar (composer, chat, history).
final class KeyPanel: NSPanel {
    override var canBecomeKey: Bool { true }

    /// A panel showing `view` on Buddy's surface; `visibleSize` is the shape's size (the window adds the margin).
    static func make<V: View>(visibleSize: CGSize, cornerRadius: CGFloat, prominent: Bool = false, view: V) -> (KeyPanel, NSHostingView<AnyView>) {
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
        let host = NSHostingView(rootView: AnyView(view.buddySurface(cornerRadius: cornerRadius, prominent: prominent)))
        // The window decides the size; SwiftUI fills it.
        host.sizingOptions = []
        panel.contentView = host
        return (panel, host)
    }
}
