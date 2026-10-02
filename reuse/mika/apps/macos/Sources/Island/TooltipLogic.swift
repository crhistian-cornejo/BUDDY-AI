import Foundation
import CoreGraphics

// The pure rules of the styled tooltip (placement, delay, text), kept free of SwiftUI so MikaTests can compile
// this file alone. Drawn by Tooltip.swift; unit-tested in Tests/TooltipTests.swift.

// MARK: - Pure logic (unit-tested in Tests/TooltipTests.swift)

/// Where the bubble goes and how big it is.
struct TooltipPlacement: Equatable {
    var origin: CGPoint
    var size: CGSize
    /// True when the bubble sits under the control; false when it flipped above it.
    var below: Bool
    /// Non-nil only when the text had to be shortened (it ends in "…") because there was no room.
    var lineLimit: Int?
}

enum TooltipGeometry {
    static let maxWidth: CGFloat = 240
    /// Space between the control and the bubble.
    static let gap: CGFloat = 6
    /// Space kept between the bubble and the panel's edge.
    static let margin: CGFloat = 4
    /// 11 pt text: one line, and the 6 pt padding above and below.
    static let lineHeight: CGFloat = 14
    static let verticalPadding: CGFloat = 12

    /// The widest the text may be in a host `hostWidth` wide.
    static func textWidthLimit(hostWidth: CGFloat) -> CGFloat {
        max(40, min(maxWidth, hostWidth - 2 * margin))
    }

    /// `anchor` is the control's rect and `bounds` the host's size, both in the host's coordinates (origin top-left);
    /// `natural` is the bubble's size with the text laid out at `textWidthLimit`.
    /// Below and centred; flips above when it does not fit below and fits above; clamped inside the bounds; when it
    /// fits neither way it takes the side with more room and shortens the text to the lines that fit.
    static func place(anchor: CGRect, natural: CGSize, bounds: CGSize) -> TooltipPlacement {
        let width = max(0, min(natural.width, bounds.width - 2 * margin))
        var height = natural.height
        let roomBelow = bounds.height - margin - anchor.maxY - gap
        let roomAbove = anchor.minY - gap - margin

        var below = true
        var lineLimit: Int?
        if height <= roomBelow {
            below = true
        } else if height <= roomAbove {
            below = false
        } else {
            below = roomBelow >= roomAbove
            let room = max(below ? roomBelow : roomAbove, 0)
            let lines = max(1, Int(((room - verticalPadding) / lineHeight).rounded(.down)))
            lineLimit = lines
            height = min(height, CGFloat(lines) * lineHeight + verticalPadding)
        }

        let rawY = below ? anchor.maxY + gap : anchor.minY - gap - height
        let rawX = anchor.midX - width / 2
        let maxX = bounds.width - margin - width
        let maxY = bounds.height - margin - height
        let x = maxX < margin ? margin : min(max(rawX, margin), maxX)
        let y = maxY < margin ? margin : min(max(rawY, margin), maxY)
        return TooltipPlacement(origin: CGPoint(x: x, y: y), size: CGSize(width: width, height: height),
                                below: below, lineLimit: lineLimit)
    }
}

enum TooltipTiming {
    /// Seconds from entering a control to showing its tip.
    static let delay: TimeInterval = 0.45
    /// A tip shown (or hidden) this recently makes the next one instant, so moving along a row of icons feels fluid.
    static let warmWindow: TimeInterval = 0.6

    /// `lastVisibleAt` is when a tip was last on screen (shown or hidden); nil if never.
    static func delay(now: Date, lastVisibleAt: Date?) -> TimeInterval {
        guard let last = lastVisibleAt, now.timeIntervalSince(last) < warmWindow else { return delay }
        return 0
    }
}

enum TooltipText {
    /// The text a tip shows: trimmed; nil (no tip at all) when it is empty, e.g. `.tip(card.error ?? "")`.
    static func resolve(_ raw: String) -> String? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    /// The keycap text: nil when empty.
    static func resolveShortcut(_ raw: String?) -> String? {
        guard let raw else { return nil }
        return resolve(raw)
    }
}
