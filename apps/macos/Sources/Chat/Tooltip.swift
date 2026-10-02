// Ported from MIKA (MIT, © MIKA contributors, revision d050bc5): apps/macos/Sources/Island/Tooltip.swift
import SwiftUI
import AppKit

// App-wide styled tooltip. System `.help` tooltips often do not show in the app's non-activating borderless panels
// and always look foreign, so every control says `.tip("Texto")` instead (call-compatible with `.help`).
//
// How it works: the control only tracks hover (`onHover`) and, while its tip is due, reports its bounds and text
// through a preference. The root view of each panel (`.tooltipHost()`) draws the bubble as an overlay, so no
// `.clipShape` / `.clipped()` parent between the control and the root can cut it. One shared `TooltipCenter` owns the
// delay and the "only one at a time" rule. When nothing is hovered there is no timer, no task, no event monitor.
// Spec twin: the Windows tooltip (docs/SPEC.md, "Tooltips").

// MARK: - The control's side

extension View {
    /// A styled tooltip. Same call shape as `.help(_:)`. `iconOnly: true` for controls that have no text of their own:
    /// the tip then also becomes the accessibility label (otherwise it is the accessibility hint).
    func tip(_ text: String, shortcut: String? = nil, iconOnly: Bool = false) -> some View {
        modifier(TooltipModifier(text: TooltipText.resolve(text), shortcut: TooltipText.resolveShortcut(shortcut),
                                 iconOnly: iconOnly))
    }

    /// Put this on the root view of every panel/window that has controls with `.tip`; it draws the bubble.
    func tooltipHost() -> some View {
        modifier(TooltipHostModifier())
    }
}

struct TooltipEntry {
    let id: UUID
    let text: String
    let shortcut: String?
    let anchor: Anchor<CGRect>
}

struct TooltipPreferenceKey: PreferenceKey {
    static var defaultValue: [TooltipEntry] { [] }
    static func reduce(value: inout [TooltipEntry], nextValue: () -> [TooltipEntry]) {
        value.append(contentsOf: nextValue())
    }
}

struct TooltipModifier: ViewModifier {
    let text: String?
    let shortcut: String?
    let iconOnly: Bool
    @State private var id = UUID()
    @State private var visible = false

    func body(content: Content) -> some View {
        let shown = visible
        let id = self.id
        let text = self.text
        let shortcut = self.shortcut
        return content
            .onHover { inside in
                guard text != nil else { return }
                if inside {
                    TooltipCenter.shared.hoverBegan(id: id, show: { visible = true }, hide: { visible = false })
                } else {
                    TooltipCenter.shared.hoverEnded(id: id)
                }
            }
            .anchorPreference(key: TooltipPreferenceKey.self, value: .bounds) { anchor in
                guard shown, let text else { return [] }
                return [TooltipEntry(id: id, text: text, shortcut: shortcut, anchor: anchor)]
            }
            .onDisappear { TooltipCenter.shared.dismiss(id: id) }
            .modifier(TooltipAccessibility(text: text, iconOnly: iconOnly))
    }
}

private struct TooltipAccessibility: ViewModifier {
    let text: String?
    let iconOnly: Bool

    @ViewBuilder
    func body(content: Content) -> some View {
        if let text {
            if iconOnly { content.accessibilityLabel(text) } else { content.accessibilityHint(text) }
        } else {
            content
        }
    }
}

// MARK: - The shared brain

/// One tip at a time. A `Task` exists only between entering a control and its tip showing; the event monitor
/// (any click, scroll or key hides the tip) exists only while a control is hovered. Idle: nothing.
@MainActor
final class TooltipCenter {
    static let shared = TooltipCenter()

    private var task: Task<Void, Never>?
    private var pendingID: UUID?
    private var visibleID: UUID?
    private var hideVisible: (() -> Void)?
    private var lastVisibleAt: Date?
    private var monitor: Any?

    func hoverBegan(id: UUID, show: @escaping () -> Void, hide: @escaping () -> Void) {
        cancelPending()
        if visibleID != nil, visibleID != id { hideCurrent() }
        pendingID = id
        installMonitor()
        let wait = TooltipTiming.delay(now: Date(), lastVisibleAt: lastVisibleAt)
        if wait <= 0 {
            reveal(id: id, show: show, hide: hide)
            return
        }
        task = Task { [weak self] in
            try? await Task.sleep(nanoseconds: UInt64(wait * 1_000_000_000))
            guard !Task.isCancelled, let self, self.pendingID == id else { return }
            self.reveal(id: id, show: show, hide: hide)
        }
    }

    func hoverEnded(id: UUID) {
        if pendingID == id { cancelPending() }
        if visibleID == id { hideCurrent() }
        if pendingID == nil, visibleID == nil { removeMonitor() }
    }

    /// The control went away (or was pressed): same as leaving it.
    func dismiss(id: UUID) {
        hoverEnded(id: id)
    }

    func dismissAll() {
        cancelPending()
        hideCurrent()
        removeMonitor()
    }

    private func reveal(id: UUID, show: () -> Void, hide: @escaping () -> Void) {
        task = nil
        pendingID = nil
        visibleID = id
        hideVisible = hide
        lastVisibleAt = Date()
        show()
    }

    private func hideCurrent() {
        guard visibleID != nil else { return }
        let hide = hideVisible
        visibleID = nil
        hideVisible = nil
        lastVisibleAt = Date()
        hide?()
    }

    private func cancelPending() {
        task?.cancel()
        task = nil
        pendingID = nil
    }

    private func installMonitor() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .scrollWheel, .keyDown]) { event in
            MainActor.assumeIsolated { TooltipCenter.shared.dismissAll() }
            return event
        }
    }

    private func removeMonitor() {
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
    }
}

// MARK: - The root's side

struct TooltipHostModifier: ViewModifier {
    func body(content: Content) -> some View {
        content.overlayPreferenceValue(TooltipPreferenceKey.self) { entries in
            GeometryReader { proxy in
                TooltipLayer(entry: entries.last, bounds: proxy.size,
                             rect: entries.last.map { proxy[$0.anchor] })
            }
            .allowsHitTesting(false)
        }
    }
}

private enum TooltipStyle {
    // Inverted against the window, like the system's: dark on a light window, light on a dark one.
    static let ink = Color.dynamic(light: "#FFFFFF", dark: "#1C1C1E")
    static let fill = Color.dynamic(light: "#1C1C1E", dark: "#F2F2F7").opacity(0.96)
    static let fade: Double = 0.12
}

private struct TooltipSizeKey: PreferenceKey {
    static var defaultValue: CGSize { .zero }
    static func reduce(value: inout CGSize, nextValue: () -> CGSize) {
        let next = nextValue()
        if next != .zero { value = next }
    }
}

private struct TooltipLayer: View {
    let entry: TooltipEntry?
    let bounds: CGSize
    let rect: CGRect?
    @State private var measured: CGSize = .zero
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        ZStack(alignment: .topLeading) {
            if let entry, let rect {
                let limit = TooltipGeometry.textWidthLimit(hostWidth: bounds.width)
                // Invisible copy at the natural size: measuring it does not depend on how the shown one is trimmed.
                TooltipBubble(text: entry.text, shortcut: entry.shortcut, maxWidth: limit, lineLimit: nil)
                    .background(GeometryReader { Color.clear.preference(key: TooltipSizeKey.self, value: $0.size) })
                    .opacity(0)
                    .accessibilityHidden(true)
                if measured != .zero {
                    let placed = TooltipGeometry.place(anchor: rect, natural: measured, bounds: bounds)
                    TooltipBubble(text: entry.text, shortcut: entry.shortcut, maxWidth: placed.size.width,
                                  lineLimit: placed.lineLimit)
                        .offset(x: placed.origin.x, y: placed.origin.y)
                        .id(entry.id)
                        .transition(reduceMotion ? .identity : .opacity)
                        .accessibilityHidden(true)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .onPreferenceChange(TooltipSizeKey.self) { size in
            measured = CGSize(width: ceil(size.width), height: ceil(size.height))
        }
        .animation(reduceMotion ? nil : .easeOut(duration: TooltipStyle.fade), value: entry?.id)
    }
}

/// As wide as its content wants, but never wider than `maxWidth` (then the text wraps). `frame(maxWidth:)` is not that: it
/// takes all the width it is offered, up to the maximum, which made every tip as wide as the limit.
private struct HugLayout: Layout {
    var maxWidth: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        subviews.first?.sizeThatFits(ProposedViewSize(width: min(maxWidth, proposal.width ?? maxWidth), height: nil)) ?? .zero
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        subviews.first?.place(at: bounds.origin, anchor: .topLeading, proposal: ProposedViewSize(width: bounds.width, height: nil))
    }
}

struct TooltipBubble: View {
    let text: String
    let shortcut: String?
    let maxWidth: CGFloat
    let lineLimit: Int?

    var body: some View {
        HugLayout(maxWidth: maxWidth) {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(verbatim: text)
                .font(.system(size: 11, weight: .medium))
                .foregroundColor(TooltipStyle.ink)
                .lineLimit(lineLimit)
                .truncationMode(.tail)
                .multilineTextAlignment(.leading)
                .fixedSize(horizontal: false, vertical: true)
            if let shortcut {
                Text(verbatim: shortcut)
                    .font(.system(size: 10, weight: .medium))
                    .foregroundColor(TooltipStyle.ink.opacity(0.8))
                    .padding(.horizontal, 4)
                    .padding(.vertical, 1)
                    .background(RoundedRectangle(cornerRadius: 4).fill(Color.white.opacity(0.1)))
                    .fixedSize()
            }
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 6)
        }
        .background(
            RoundedRectangle(cornerRadius: 9, style: .continuous)
                .fill(TooltipStyle.fill)
                .overlay(RoundedRectangle(cornerRadius: 9, style: .continuous)
                    .stroke(Color.white.opacity(0.1), lineWidth: 1))
                .shadow(color: .black.opacity(0.16), radius: 4, x: 0, y: 1)
        )
        .fixedSize(horizontal: false, vertical: true)
    }
}
