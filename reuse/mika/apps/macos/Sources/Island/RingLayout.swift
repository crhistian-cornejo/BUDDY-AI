import Foundation
import CoreGraphics

// Where the items of the pet's ring go (twin of `ringLayout` / `ringDirection` / `ringProblems` in
// apps/windows/src/pet/layout.ts, same numbers and same search). Pure, so it is tested without a screen.
//
// Items: Música first (only while a player has something), then the agents, then Settings last, all on one arc with equal
// angle steps (the two ends included) and one radius searched from 96 to 134 pt in 2 pt steps; the first layout whose
// items fit wins. Labelled items (icon plus its name, a 64 x 52 box) are tried first; where they do not fit (corners, many
// items) the items are bare 30 pt circles and the name shows only on the hovered one. Every coordinate is in the ring
// window's own space, y down, in points.

enum RingArc: Equatable { case up, down, left, right }

/// How far Mika's body reaches from the ring's centre, per side. The Windows mascot is 96 pt wide and only its rows 32...80
/// are drawn (so 48 / 48 / 16 / 32); the Mac's circle is a 34 pt hover disc.
struct RingBody: Equatable {
    var left: CGFloat
    var right: CGFloat
    var up: CGFloat
    var down: CGFloat

    static let circle = RingBody(left: 17, right: 17, up: 17, down: 17)
    static let windowsMika = RingBody(left: 48, right: 48, up: 16, down: 32)
}

/// The ring window and where the mascot is in it.
struct RingFrame: Equatable {
    var width: CGFloat
    var height: CGFloat
    var cx: CGFloat
    var cy: CGFloat
    /// Which way the half circle faces when no screen edge is near (Rust's `frame_for` on Windows: away from the nearest edge).
    var arc: RingArc = .down
    var body: RingBody = .circle
}

struct RingBox: Equatable {
    var left: CGFloat
    var top: CGFloat
    var right: CGFloat
    var bottom: CGFloat

    func overlaps(_ other: RingBox, pad: CGFloat = 0) -> Bool {
        left < other.right + pad && other.left < right + pad && top < other.bottom + pad && other.top < bottom + pad
    }
}

struct RingSpots: Equatable {
    var agents: [CGPoint]
    var music: CGPoint?
    var settings: CGPoint?
    var compact: Bool
    var radius: CGFloat
    /// 1 = one radius; 2 = two alternating radii (only when nothing else fits).
    var rings: Int
    /// The arc that was used (the middle direction and the width, in radians; y down).
    var facing: CGFloat
    var span: CGFloat

    /// Música, the agents, Settings: the order of the arc.
    var all: [CGPoint] {
        var out: [CGPoint] = []
        if let music { out.append(music) }
        out.append(contentsOf: agents)
        if let settings { out.append(settings) }
        return out
    }
}

enum PetRing {
    /// One labelled item around its anchor (the icon circle's centre): a 64 x 52 box that starts 32 left of and 21 above it.
    static let itemLeft: CGFloat = -32
    static let itemTop: CGFloat = -21
    static let itemWidth: CGFloat = 64
    static let itemHeight: CGFloat = 52
    /// A bare item: a 30 pt circle centred on the anchor.
    static let iconSize: CGFloat = 30
    /// The Mac's ring window: wide enough for a radius of 190 and its labels, as tall as the farthest item needs.
    static let windowWidth: CGFloat = 340
    static let windowHeight: CGFloat = 260
    /// Distance to a screen edge under which that edge counts as near (the window is 340 wide and centred on the mascot
    /// unless an edge pushes it: a centre closer than 168 means an edge is near).
    static let near: CGFloat = 168
    /// Where the Mac's half circle may be narrowed: the circle sits 17 pt under the top edge, so an item level with it would
    /// have half its box above the screen. Windows (a mascot 48 pt from the edge) never needs this.
    static let macNarrowing: [CGFloat] = [0.92, 0.84, 0.76, 0.68]

    static func itemBox(_ p: CGPoint) -> RingBox {
        RingBox(left: p.x + itemLeft, top: p.y + itemTop,
                right: p.x + itemLeft + itemWidth, bottom: p.y + itemTop + itemHeight)
    }

    static func iconBox(_ p: CGPoint) -> RingBox {
        RingBox(left: p.x - iconSize / 2, top: p.y - iconSize / 2, right: p.x + iconSize / 2, bottom: p.y + iconSize / 2)
    }

    static func bodyBox(_ f: RingFrame) -> RingBox {
        RingBox(left: f.cx - f.body.left, top: f.cy - f.body.up, right: f.cx + f.body.right, bottom: f.cy + f.body.down)
    }

    /// Violations of one layout: items outside the window (4 pt margin), touching each other (boxes 2 pt apart, or circle
    /// centres 34 pt apart in `compact`), or within 4 pt of the mascot's body.
    static func problems(_ f: RingFrame, _ points: [CGPoint], compact: Bool = false) -> Int {
        let edge: CGFloat = 4
        let body = bodyBox(f)
        var bad = 0
        for (i, p) in points.enumerated() {
            let b = itemBox(p)
            if b.left < edge || b.top < edge || b.right > f.width - edge || b.bottom > f.height - edge { bad += 1 }
            if (compact ? iconBox(p) : b).overlaps(body, pad: 4) { bad += 1 }
            var j = i + 1
            while j < points.count {
                let q = points[j]
                if compact {
                    if hypot(p.x - q.x, p.y - q.y) < iconSize + 4 { bad += 1 }
                } else if b.overlaps(itemBox(q), pad: 2) {
                    bad += 1
                }
                j += 1
            }
        }
        return bad
    }

    /// Where the ring faces (the angle of the arc's middle, y down) and how wide it is: near both axes a quarter circle
    /// facing into the screen, near one a half circle facing away from that edge, near none a half circle toward `f.arc`.
    static func direction(_ f: RingFrame) -> (facing: CGFloat, span: CGFloat) {
        let left = f.cx, right = f.width - f.cx, top = f.cy, bottom = f.height - f.cy
        let h: CGFloat = min(left, right) < near ? (left <= right ? 1 : -1) : 0   // +1: edge on the left, ring opens to the right
        let v: CGFloat = min(top, bottom) < near ? (top <= bottom ? 1 : -1) : 0
        func deg(_ a: CGFloat) -> CGFloat { a * .pi / 180 }
        if h != 0 && v != 0 { return (atan2(v, h), deg(90)) }
        if h != 0 { return (h > 0 ? 0 : deg(180), deg(180)) }
        if v != 0 { return (v > 0 ? deg(90) : deg(-90), deg(180)) }
        switch f.arc {
        case .up: return (deg(-90), deg(180))
        case .down: return (deg(90), deg(180))
        case .left: return (deg(180), deg(180))
        case .right: return (0, deg(180))
        }
    }

    /// `count` points evenly spaced over an arc (ends included); `rings` > 1 alternates radii (outer first).
    private static func arcPoints(_ f: RingFrame, count: Int, facing: CGFloat, span: CGFloat,
                                  outer: CGFloat, rings: Int, gap: CGFloat) -> [CGPoint] {
        (0..<count).map { i in
            let angle = count == 1 ? facing : facing - span / 2 + CGFloat(i) * span / CGFloat(count - 1)
            let radius = outer - CGFloat(i % rings) * gap
            return CGPoint(x: f.cx + cos(angle) * radius, y: f.cy + sin(angle) * radius)
        }
    }

    private struct Stage {
        let compact: Bool
        let rings: Int
        let gap: CGFloat
        let from: CGFloat
        let to: CGFloat
    }

    /// The first layout with every item inside the window, apart from each other and clear of the body: arcs (the natural
    /// width, any `narrower` fractions of it, then 1.5x, 2x and a full turn, turned 0 / +45 / -45 degrees) x stages
    /// (labelled one radius 96...134, bare one radius, bare two radii 38 apart, bare one radius 136...170, bare two radii
    /// up to 190) x radii in 2 pt steps. If nothing fits, the layout with the fewest violations.
    static func layout(_ f: RingFrame, agents: Int, music: Bool, settings: Bool = true,
                       narrower: [CGFloat] = []) -> RingSpots {
        let lead = music ? 1 : 0
        let count = agents + lead + (settings ? 1 : 0)
        let (facing, span) = direction(f)
        guard count > 0 else {
            return RingSpots(agents: [], music: nil, settings: nil, compact: false, radius: 0, rings: 1, facing: facing, span: span)
        }
        func split(_ pts: [CGPoint], _ compact: Bool, _ radius: CGFloat, _ rings: Int, _ dir: CGFloat, _ wide: CGFloat) -> RingSpots {
            RingSpots(agents: Array(pts[lead..<(lead + agents)]), music: music ? pts[0] : nil,
                      settings: settings ? pts[count - 1] : nil, compact: compact, radius: radius, rings: rings,
                      facing: dir, span: wide)
        }
        let quarter = CGFloat.pi / 4
        let fullTurn: CGFloat = 2 * CGFloat.pi
        var spans: [CGFloat] = [span]
        for k in narrower { spans.append(span * k) }
        spans.append(min(span * 1.5, fullTurn))
        spans.append(min(span * 2, fullTurn))
        spans.append(fullTurn)
        let stages = [
            Stage(compact: false, rings: 1, gap: 0, from: 96, to: 134),
            Stage(compact: true, rings: 1, gap: 0, from: 96, to: 134),
            Stage(compact: true, rings: 2, gap: 38, from: 120, to: 134),
            Stage(compact: true, rings: 1, gap: 0, from: 136, to: 170),
            Stage(compact: true, rings: 2, gap: 38, from: 136, to: 190),
        ]
        var best: RingSpots?
        var bestBad = Int.max
        for wide in spans {
            let turns: [CGFloat] = wide <= span ? [0] : [0, quarter, -quarter]
            for turn in turns {
                let dir = facing + turn
                for stage in stages {
                    var outer = stage.from
                    while outer <= stage.to {
                        let pts = arcPoints(f, count: count, facing: dir, span: wide, outer: outer, rings: stage.rings, gap: stage.gap)
                        let bad = problems(f, pts, compact: stage.compact)
                        if bad == 0 { return split(pts, stage.compact, outer, stage.rings, dir, wide) }
                        if bad < bestBad {
                            best = split(pts, stage.compact, outer, stage.rings, dir, wide)
                            bestBad = bad
                        }
                        outer += 2
                    }
                }
            }
        }
        return best ?? RingSpots(agents: [], music: nil, settings: nil, compact: false, radius: 0, rings: 1, facing: facing, span: span)
    }

    /// The Mac's ring: a window `windowWidth` x `windowHeight` flush with the top of `screen` and centred on the circle
    /// (pushed inside the screen when the circle is near a side), and the circle's place in it. `circle` is the circle's
    /// centre in screen coordinates (origin bottom-left). The circle sits in the menu bar strip, so a screen edge is always
    /// near at the top and the half circle fans out below it.
    static func macFrame(circle: CGPoint, screen: CGRect) -> (window: CGRect, ring: RingFrame) {
        let width = min(windowWidth, screen.width), height = min(windowHeight, screen.height)
        let x = max(screen.minX, min(circle.x - width / 2, screen.maxX - width))
        let window = CGRect(x: x, y: screen.maxY - height, width: width, height: height)
        let ring = RingFrame(width: width, height: height, cx: circle.x - x, cy: screen.maxY - circle.y,
                             arc: .down, body: .circle)
        return (window, ring)
    }
}
