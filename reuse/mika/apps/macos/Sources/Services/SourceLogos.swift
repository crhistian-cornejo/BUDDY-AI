import Foundation
import CoreGraphics

// Site logos for the sources row under an answer. The most common sites get a small mark drawn in code (24x24 vector shapes,
// no bitmaps, no network); any other site gets a letter tile whose colour comes from a hash of its domain. Hosts are
// matched by their registrable domain (`developer.mozilla.org` -> `mozilla.org`, `news.bbc.co.uk` -> `bbc.co.uk`). Twin of
// apps/windows/src/core/logos.ts: same marks (the shape data is copied from it), same domain rules, same fallback colour.
// Pure, so it runs under XCTest without a screen; `SourceChip` (ChatMarkdownView.swift) draws a `SiteLogo` with SwiftUI.

/// One vector shape in a 24x24 box: filled (default white) unless it has a stroke. `d` is SVG path data.
struct LogoShape: Equatable, Sendable {
    var d: String
    var fill: String?
    var stroke: String?
    var width: Double

    static func filled(_ d: String, _ color: String = "#ffffff") -> LogoShape {
        LogoShape(d: d, fill: color, stroke: nil, width: 0)
    }

    static func stroked(_ d: String, _ color: String = "#ffffff", _ width: Double = 1.8) -> LogoShape {
        LogoShape(d: d, fill: nil, stroke: color, width: width)
    }
}

/// A mark by id: the tile's background and its shapes.
struct LogoMark: Equatable, Sendable {
    var bg: String
    var shapes: [LogoShape]
}

/// The drawn logo of a site.
struct SiteLogo: Equatable, Sendable {
    var id: String
    var bg: String
    var shapes: [LogoShape]
}

enum SourceLogos {
    /// Marks by id. Simple brand-colour shapes, drawn from scratch.
    static let marks: [String: LogoMark] = [
        "google": LogoMark(bg: "#ffffff", shapes: [
            .stroked("M16.6 7.4A6.5 6.5 0 0 0 5.5 12", "#ea4335", 3),
            .stroked("M5.5 12A6.5 6.5 0 0 0 8.7 17.6", "#fbbc05", 3),
            .stroked("M8.7 17.6A6.5 6.5 0 0 0 17.6 15.25", "#34a853", 3),
            .stroked("M17.6 15.25A6.5 6.5 0 0 0 18.5 12H12", "#4285f4", 3),
        ]),
        "wikipedia": LogoMark(bg: "#ffffff", shapes: [
            .stroked("M4.5 7.5l3.2 9.5 4.3-8.5 4.3 8.5 3.2-9.5", "#111111", 1.7),
        ]),
        "github": LogoMark(bg: "#161b22", shapes: [
            .filled("M12 5.5c-3.6 0-6 2.3-6 5.4 0 2.4 1.4 4.3 3.4 5.1v1.5h5.2V16c2-.8 3.4-2.7 3.4-5.1 0-3.1-2.4-5.4-6-5.4z", "#ffffff"),
            .filled("M6.8 5.2l2.3 1.2-2.1 1.6zM17.2 5.2l-2.3 1.2 2.1 1.6z", "#ffffff"),
        ]),
        "stackoverflow": LogoMark(bg: "#f48024", shapes: [
            .stroked("M6.5 14v4.5h11V14", "#ffffff", 1.8),
            .stroked("M8.5 17h7", "#ffffff", 1.6),
            .stroked("M8.8 14.2l7-1.4M9.7 11.4l6.4-2.9M11.7 8.9l5.4-4", "#ffffff", 1.6),
        ]),
        "youtube": LogoMark(bg: "#ffffff", shapes: [
            .filled("M4 8.2A2.2 2.2 0 0 1 6.2 6h11.6A2.2 2.2 0 0 1 20 8.2v7.6a2.2 2.2 0 0 1-2.2 2.2H6.2A2.2 2.2 0 0 1 4 15.8z", "#ff0000"),
            .filled("M10.2 9.2v5.6l4.8-2.8z", "#ffffff"),
        ]),
        "x": LogoMark(bg: "#000000", shapes: [
            .stroked("M7 6.5l10 11M17 6.5L7 17.5", "#ffffff", 2),
        ]),
        "reddit": LogoMark(bg: "#ff4500", shapes: [
            .filled("M12 9.2c-3.6 0-6.3 1.9-6.3 4.4 0 2.5 2.7 4.4 6.3 4.4s6.3-1.9 6.3-4.4c0-2.5-2.7-4.4-6.3-4.4z", "#ffffff"),
            .filled("M8.1 13.4a1.1 1.1 0 1 0 2.2 0 1.1 1.1 0 1 0-2.2 0zM13.7 13.4a1.1 1.1 0 1 0 2.2 0 1.1 1.1 0 1 0-2.2 0z", "#ff4500"),
            .stroked("M12 9.2l1-3.6 3 .8", "#ffffff", 1.1),
            .filled("M16.3 6.4a1.1 1.1 0 1 0 2.2 0 1.1 1.1 0 1 0-2.2 0z", "#ffffff"),
        ]),
        "mlssoccer": LogoMark(bg: "#0d1f3c", shapes: [
            .stroked("M5 12a7 7 0 1 0 14 0 7 7 0 1 0-14 0z", "#ffffff", 1.2),
            .filled("M12 9.4l2.4 1.8-.9 2.8h-3l-.9-2.8z", "#ffffff"),
            .stroked("M12 9.4V5.3M14.4 11.2l3.6-1.2M13.5 14l2.2 3.1M10.5 14l-2.2 3.1M9.6 11.2L6 10", "#ffffff", 1.1),
        ]),
        "espn": LogoMark(bg: "#d00000", shapes: [
            .filled("M7 7h10v2.4H9.6v1.3h6.6v2.4H9.6v1.5H17V17H7z", "#ffffff"),
        ]),
        "bbc": LogoMark(bg: "#ffffff", shapes: [
            .filled("M4.5 9h4.4v6H4.5zM9.8 9h4.4v6H9.8zM15.1 9h4.4v6h-4.4z", "#111111"),
        ]),
        "nytimes": LogoMark(bg: "#ffffff", shapes: [
            .filled("M6.5 6.5h11v2.2h-4.2v8.8h-2.6V8.7H6.5z", "#111111"),
        ]),
        "mozilla": LogoMark(bg: "#000000", shapes: [
            .stroked("M6 17.5V7l6 6.5L18 7v10.5", "#ffffff", 2),
        ]),
        "microsoft": LogoMark(bg: "#ffffff", shapes: [
            .filled("M5.5 5.5h6.3v6.3H5.5z", "#f25022"),
            .filled("M12.2 5.5h6.3v6.3h-6.3z", "#7fba00"),
            .filled("M5.5 12.2h6.3v6.3H5.5z", "#00a4ef"),
            .filled("M12.2 12.2h6.3v6.3h-6.3z", "#ffb900"),
        ]),
        "apple": LogoMark(bg: "#000000", shapes: [
            .filled("M12 8.5c1.2-1 3.8-.9 5 1-2 1.2-1.6 4 .6 4.8-.6 1.6-1.9 3.7-3.2 3.7-.8 0-1.2-.5-2.4-.5s-1.6.5-2.4.5c-1.4 0-3.4-3-3.4-5.5 0-2.4 1.5-4 3.4-4 1 0 1.8.5 2.4 0z", "#ffffff"),
            .filled("M12.2 7.6c0-1.3.9-2.4 2.2-2.6 0 1.3-.9 2.4-2.2 2.6z", "#ffffff"),
        ]),
        "anthropic": LogoMark(bg: "#cc785c", shapes: [
            .stroked("M7 18L12 6l5 12", "#ffffff", 2.2),
            .stroked("M9 14h6", "#ffffff", 2),
        ]),
        "openai": LogoMark(bg: "#000000", shapes: [
            .stroked("M12 5.5l5.6 3.25v6.5L12 18.5l-5.6-3.25v-6.5z", "#ffffff", 1.6),
            .stroked("M12 9l3 1.75v3.5L12 16l-3-1.75v-3.5z", "#ffffff", 1.2),
        ]),
        "medium": LogoMark(bg: "#000000", shapes: [
            .filled("M5.4 8.2a3.8 3.8 0 1 0 7.6 0 3.8 3.8 0 1 0-7.6 0z", "#ffffff"),
            .filled("M15.6 8.8c1 0 1.7 1.5 1.7 3.2s-.7 3.2-1.7 3.2-1.7-1.5-1.7-3.2.7-3.2 1.7-3.2z", "#ffffff"),
            .filled("M18.6 8.8h.9v6.4h-.9z", "#ffffff"),
        ]),
        "linkedin": LogoMark(bg: "#0a66c2", shapes: [
            .filled("M6.1 8.1a1.3 1.3 0 1 0 2.6 0 1.3 1.3 0 1 0-2.6 0z", "#ffffff"),
            .filled("M6.2 10.2h2.4v7.6H6.2z", "#ffffff"),
            .filled("M10.4 10.2h2.3v1.1c.4-.7 1.2-1.3 2.5-1.3 2.2 0 2.8 1.4 2.8 3.4v4.4h-2.4v-3.9c0-1-.2-1.8-1.3-1.8s-1.5.8-1.5 1.8v3.9h-2.4z", "#ffffff"),
        ]),
        "amazon": LogoMark(bg: "#232f3e", shapes: [
            .stroked("M14.6 12.6a3 3 0 1 1-6 0 3 3 0 0 1 6 0z", "#ffffff", 1.8),
            .stroked("M14.6 9.8v5.4", "#ffffff", 1.8),
            .stroked("M6.5 17.8c3.5 2 8 2 11.5-.2", "#ff9900", 1.6),
        ]),
        "flashscore": LogoMark(bg: "#e0003c", shapes: [
            .filled("M13.2 4.5L7.5 13h3.8l-1 6.5 6.2-9h-4z", "#ffffff"),
        ]),
        "sofascore": LogoMark(bg: "#374df5", shapes: [
            .stroked("M16.5 8.5c-.8-1.6-2.4-2.3-4.4-2.1-2.1.2-3.3 1.8-2.7 3.2.7 1.6 3.6 1.7 4.8 2.7 1.4 1.1.9 3-.8 3.6-2 .7-4 0-5-1.7", "#ffffff", 2.2),
        ]),
        "transfermarkt": LogoMark(bg: "#1a3151", shapes: [
            .stroked("M6.5 9h11M14.5 6l3 3-3 3M17.5 15h-11M9.5 12l-3 3 3 3", "#ffffff", 1.8),
        ]),
        "betano": LogoMark(bg: "#ff6a13", shapes: [
            .stroked("M9 5.5v13", "#ffffff", 2.4),
            .stroked("M9 11.5h2.6a3.5 3.5 0 1 1 0 7H9", "#ffffff", 2.4),
        ]),
        "footballdata": LogoMark(bg: "#1f7a3a", shapes: [
            .stroked("M5 7.5h14v9H5zM12 7.5v9", "#ffffff", 1.3),
            .stroked("M10.2 12a1.8 1.8 0 1 0 3.6 0 1.8 1.8 0 1 0-3.6 0z", "#ffffff", 1.3),
        ]),
        "tennismylife": LogoMark(bg: "#c9e03a", shapes: [
            .stroked("M7 6.5c3 2.5 3 8.5 0 11M17 6.5c-3 2.5-3 8.5 0 11", "#ffffff", 1.8),
        ]),
        "facebook": LogoMark(bg: "#1877f2", shapes: [
            .filled("M13.4 19v-6h2l.4-2.4h-2.4V9.2c0-.7.3-1.2 1.3-1.2h1.2V5.8c-.3 0-1-.1-1.8-.1-1.8 0-3 1.1-3 3.1v1.8H9v2.4h2.1v6z", "#ffffff"),
        ]),
    ]

    /// Registrable domain -> logo id.
    static let domains: [String: String] = [
        "google.com": "google", "wikipedia.org": "wikipedia", "github.com": "github", "stackoverflow.com": "stackoverflow",
        "stackexchange.com": "stackoverflow", "youtube.com": "youtube", "youtu.be": "youtube", "x.com": "x", "twitter.com": "x",
        "reddit.com": "reddit", "redd.it": "reddit", "mlssoccer.com": "mlssoccer", "espn.com": "espn", "bbc.com": "bbc",
        "bbc.co.uk": "bbc", "nytimes.com": "nytimes", "mozilla.org": "mozilla", "microsoft.com": "microsoft",
        "apple.com": "apple", "anthropic.com": "anthropic", "claude.ai": "anthropic", "openai.com": "openai",
        "chatgpt.com": "openai", "medium.com": "medium", "linkedin.com": "linkedin", "amazon.com": "amazon",
        "flashscore.com": "flashscore", "sofascore.com": "sofascore", "transfermarkt.com": "transfermarkt",
        "betano.com": "betano", "football-data.org": "footballdata", "tennismylife.org": "tennismylife",
        "facebook.com": "facebook",
    ]

    /// The same brand under its country domains (`google.com.pe`, `amazon.es`, `espn.com.ar`). Only generic or country
    /// endings count, never `google.xyz`.
    static let brands: [String: String] = [
        "google": "google", "amazon": "amazon", "espn": "espn", "bbc": "bbc", "flashscore": "flashscore",
        "transfermarkt": "transfermarkt", "betano": "betano",
    ]

    /// `#rrggbb` (what the marks use; `Color(hex:)` reads exactly this).
    static func isHex(_ value: String) -> Bool {
        let bytes = Array(value.utf8)
        return bytes.count == 7 && bytes[0] == UInt8(ascii: "#")
            && bytes[1...].allSatisfy { ($0 >= 48 && $0 <= 57) || ($0 >= 97 && $0 <= 102) || ($0 >= 65 && $0 <= 70) }
    }

    /// `com`, `org`, `net`, a two-letter country code, or `com.xx` / `co.xx` / `org.xx` / `net.xx` / `gov.xx` / `ac.xx`.
    static func isBrandEnding(_ ending: String) -> Bool {
        if ["com", "org", "net"].contains(ending) { return true }
        func letters(_ s: Substring) -> Bool { s.count == 2 && s.utf8.allSatisfy { $0 >= 97 && $0 <= 122 } }
        let parts = ending.split(separator: ".", omittingEmptySubsequences: false)
        if parts.count == 1 { return letters(parts[0]) }
        return parts.count == 2 && ["com", "co", "org", "net", "gov", "ac"].contains(String(parts[0])) && letters(parts[1])
    }

    /// Lower case, no `www.`, no trailing dot.
    static func normalizeHost(_ host: String) -> String {
        var h = host.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if h.hasSuffix(".") { h.removeLast() }
        if h.hasPrefix("www.") { h.removeFirst(4) }
        return h
    }

    /// `news.bbc.co.uk` -> `bbc.co.uk`, `a.b.example.com` -> `example.com`; an IP address or a single label stays as it is.
    static func registrableDomain(_ host: String) -> String {
        let h = normalizeHost(host)
        let onlyDigitsAndDots = !h.isEmpty && h.utf8.allSatisfy { ($0 >= 48 && $0 <= 57) || $0 == 0x2E }
        if !h.contains(".") || onlyDigitsAndDots || h.contains(":") { return h }
        let parts = h.split(separator: ".", omittingEmptySubsequences: false).map(String.init)
        guard parts.count >= 2 else { return h }
        let sld = parts[parts.count - 2], tld = parts[parts.count - 1]
        let twoLevel = parts.count >= 3 && tld.count == 2 && ["co", "com", "org", "net", "gov", "edu", "ac"].contains(sld)
        return parts.suffix(twoLevel ? 3 : 2).joined(separator: ".")
    }

    /// The drawn logo for a host, or nil (then the tile is a letter, or the site's own icon once fetched).
    static func logo(for host: String) -> SiteLogo? {
        let domain = registrableDomain(host)
        var id = domains[domain]
        if id == nil, let dot = domain.firstIndex(of: "."), dot != domain.startIndex {
            let label = String(domain[..<dot])
            let ending = String(domain[domain.index(after: dot)...])
            if let brand = brands[label], isBrandEnding(ending) { id = brand }
        }
        guard let id, let mark = marks[id] else { return nil }
        return SiteLogo(id: id, bg: mark.bg, shapes: mark.shapes)
    }

    /// Neutral, muted tile colours for hosts with no logo.
    static let fallbackPalette = ["#5b8def", "#e0795c", "#58a97b", "#b07adb", "#d9a441", "#4aa3b5", "#d4607f", "#7a86c9"]

    /// The same colour for every subdomain of a site, and for ever: a hash of the registrable domain.
    static func fallbackColor(for host: String) -> String {
        var sum = 0
        for scalar in registrableDomain(host).unicodeScalars { sum = (sum &* 31 &+ Int(scalar.value)) & 0xFFFF }
        return fallbackPalette[sum % fallbackPalette.count]
    }

    /// The first letter or digit of the site's name (its domain), upper case; a dot if there is none.
    static func fallbackInitial(for host: String) -> String {
        let domain = registrableDomain(host)
        guard let first = domain.first(where: { $0.isLetter || $0.isNumber }) else { return "•" }
        return String(first).uppercased()
    }
}

// MARK: - SVG path data

/// A path as absolute segments in the 24x24 box: what an SVG `d` string means, with relative commands, `H` / `V`, smooth
/// curves and arcs already turned into moves, lines and cubic curves. Pure geometry, so it is tested without SwiftUI.
enum PathSegment: Equatable {
    case move(CGPoint)
    case line(CGPoint)
    case cubic(CGPoint, CGPoint, CGPoint)
    case quad(CGPoint, CGPoint)
    case close
}

enum SVGPath {
    /// The segments of `d`, or nil when the data is malformed (an unknown command, a missing number, a path that does not start
    /// with a move).
    static func parse(_ d: String) -> [PathSegment]? {
        var scanner = PathReader(Array(d.unicodeScalars))
        var out: [PathSegment] = []
        var current = CGPoint.zero
        var start = CGPoint.zero
        var lastCubicControl: CGPoint?
        var lastQuadControl: CGPoint?
        var command: Unicode.Scalar?

        func point(_ x: Double, _ y: Double, relative: Bool) -> CGPoint {
            relative ? CGPoint(x: current.x + x, y: current.y + y) : CGPoint(x: x, y: y)
        }

        while true {
            scanner.skipSeparators()
            if scanner.atEnd { break }
            if let c = scanner.peek, c.properties.isAlphabetic {
                command = c
                scanner.advance()
            } else if command == nil {
                return nil
            }
            guard let cmd = command else { return nil }
            let relative = cmd.value >= 97
            let upper = Unicode.Scalar(cmd.value & ~0x20) ?? cmd
            if out.isEmpty, upper != "M" { return nil }

            switch upper {
            case "Z":
                out.append(.close)
                current = start
                lastCubicControl = nil; lastQuadControl = nil
                command = nil                       // a number after Z needs a new command
                continue
            case "M":
                guard let x = scanner.number(), let y = scanner.number() else { return nil }
                current = point(x, y, relative: relative)
                start = current
                out.append(.move(current))
                command = relative ? "l" : "L"      // further pairs are line-tos
                lastCubicControl = nil; lastQuadControl = nil
            case "L":
                guard let x = scanner.number(), let y = scanner.number() else { return nil }
                current = point(x, y, relative: relative)
                out.append(.line(current))
                lastCubicControl = nil; lastQuadControl = nil
            case "H":
                guard let x = scanner.number() else { return nil }
                current = CGPoint(x: relative ? current.x + x : x, y: current.y)
                out.append(.line(current))
                lastCubicControl = nil; lastQuadControl = nil
            case "V":
                guard let y = scanner.number() else { return nil }
                current = CGPoint(x: current.x, y: relative ? current.y + y : y)
                out.append(.line(current))
                lastCubicControl = nil; lastQuadControl = nil
            case "C":
                guard let x1 = scanner.number(), let y1 = scanner.number(), let x2 = scanner.number(),
                      let y2 = scanner.number(), let x = scanner.number(), let y = scanner.number() else { return nil }
                let c1 = point(x1, y1, relative: relative), c2 = point(x2, y2, relative: relative)
                let end = point(x, y, relative: relative)
                out.append(.cubic(c1, c2, end))
                current = end
                lastCubicControl = c2; lastQuadControl = nil
            case "S":
                guard let x2 = scanner.number(), let y2 = scanner.number(), let x = scanner.number(),
                      let y = scanner.number() else { return nil }
                let c1 = lastCubicControl.map { CGPoint(x: 2 * current.x - $0.x, y: 2 * current.y - $0.y) } ?? current
                let c2 = point(x2, y2, relative: relative)
                let end = point(x, y, relative: relative)
                out.append(.cubic(c1, c2, end))
                current = end
                lastCubicControl = c2; lastQuadControl = nil
            case "Q":
                guard let x1 = scanner.number(), let y1 = scanner.number(), let x = scanner.number(),
                      let y = scanner.number() else { return nil }
                let c = point(x1, y1, relative: relative)
                let end = point(x, y, relative: relative)
                out.append(.quad(c, end))
                current = end
                lastQuadControl = c; lastCubicControl = nil
            case "T":
                guard let x = scanner.number(), let y = scanner.number() else { return nil }
                let c = lastQuadControl.map { CGPoint(x: 2 * current.x - $0.x, y: 2 * current.y - $0.y) } ?? current
                let end = point(x, y, relative: relative)
                out.append(.quad(c, end))
                current = end
                lastQuadControl = c; lastCubicControl = nil
            case "A":
                guard let rx = scanner.number(), let ry = scanner.number(), let rotation = scanner.number(),
                      let large = scanner.flag(), let sweep = scanner.flag(), let x = scanner.number(),
                      let y = scanner.number() else { return nil }
                let end = point(x, y, relative: relative)
                out += arc(from: current, to: end, rx: rx, ry: ry, rotation: rotation, large: large, sweep: sweep)
                current = end
                lastCubicControl = nil; lastQuadControl = nil
            default:
                return nil
            }
        }
        return out.isEmpty ? nil : out
    }

    /// An SVG elliptical arc as cubic Bezier curves (at most a quarter turn each), by the endpoint-to-centre conversion of
    /// the SVG specification (F.6.5). A zero radius is a straight line; radii too small for the chord are scaled up.
    static func arc(from p0: CGPoint, to p1: CGPoint, rx rxIn: Double, ry ryIn: Double, rotation: Double,
                    large: Bool, sweep: Bool) -> [PathSegment] {
        if p0 == p1 { return [] }
        var rx = abs(rxIn), ry = abs(ryIn)
        if rx == 0 || ry == 0 { return [.line(p1)] }
        let phi = rotation * Double.pi / 180
        let cosPhi = cos(phi), sinPhi = sin(phi)
        let dx = (Double(p0.x) - Double(p1.x)) / 2, dy = (Double(p0.y) - Double(p1.y)) / 2
        let x1p = cosPhi * dx + sinPhi * dy
        let y1p = -sinPhi * dx + cosPhi * dy
        let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry)
        if lambda > 1 { let k = lambda.squareRoot(); rx *= k; ry *= k }
        let numerator = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p
        let denominator = rx * rx * y1p * y1p + ry * ry * x1p * x1p
        var coefficient = denominator == 0 ? 0 : (max(0, numerator / denominator)).squareRoot()
        if large == sweep { coefficient = -coefficient }
        let cxp = coefficient * (rx * y1p / ry)
        let cyp = coefficient * -(ry * x1p / rx)
        let cx = cosPhi * cxp - sinPhi * cyp + (Double(p0.x) + Double(p1.x)) / 2
        let cy = sinPhi * cxp + cosPhi * cyp + (Double(p0.y) + Double(p1.y)) / 2

        func angle(_ ux: Double, _ uy: Double, _ vx: Double, _ vy: Double) -> Double {
            atan2(ux * vy - uy * vx, ux * vx + uy * vy)
        }
        let ux = (x1p - cxp) / rx, uy = (y1p - cyp) / ry
        let vx = (-x1p - cxp) / rx, vy = (-y1p - cyp) / ry
        let theta1 = angle(1, 0, ux, uy)
        var delta = angle(ux, uy, vx, vy)
        if !sweep, delta > 0 { delta -= 2 * Double.pi }
        if sweep, delta < 0 { delta += 2 * Double.pi }

        let pieces = max(1, Int((abs(delta) / (Double.pi / 2)).rounded(.up)))
        let step = delta / Double(pieces)
        let t = 4.0 / 3.0 * tan(step / 4)

        func map(_ x: Double, _ y: Double) -> CGPoint {
            CGPoint(x: cx + rx * x * cosPhi - ry * y * sinPhi, y: cy + rx * x * sinPhi + ry * y * cosPhi)
        }
        var out: [PathSegment] = []
        for i in 0..<pieces {
            let a1 = theta1 + Double(i) * step, a2 = a1 + step
            let c1 = map(cos(a1) - t * sin(a1), sin(a1) + t * cos(a1))
            let c2 = map(cos(a2) + t * sin(a2), sin(a2) - t * cos(a2))
            // The last piece ends exactly where the arc was asked to end.
            let end = i == pieces - 1 ? p1 : map(cos(a2), sin(a2))
            out.append(.cubic(c1, c2, end))
        }
        return out
    }

    /// Reads numbers and flags out of path data.
    private struct PathReader {
        private let scalars: [Unicode.Scalar]
        private var index = 0

        init(_ scalars: [Unicode.Scalar]) { self.scalars = scalars }

        var atEnd: Bool { index >= scalars.count }
        var peek: Unicode.Scalar? { index < scalars.count ? scalars[index] : nil }
        mutating func advance() { index += 1 }

        mutating func skipSeparators() {
            while let c = peek, c == " " || c == "," || c == "\n" || c == "\t" || c == "\r" { index += 1 }
        }

        private func isDigit(_ c: Unicode.Scalar) -> Bool { c.value >= 48 && c.value <= 57 }

        /// The next number: optional sign, digits, one dot, optional exponent. `1.2.3` is 1.2 then .3; `5-3` is 5 then -3.
        mutating func number() -> Double? {
            skipSeparators()
            let begin = index
            var i = index
            if i < scalars.count, scalars[i] == "-" || scalars[i] == "+" { i += 1 }
            var digits = 0
            while i < scalars.count, isDigit(scalars[i]) { i += 1; digits += 1 }
            if i < scalars.count, scalars[i] == "." {
                i += 1
                while i < scalars.count, isDigit(scalars[i]) { i += 1; digits += 1 }
            }
            guard digits > 0 else { return nil }
            if i < scalars.count, scalars[i] == "e" || scalars[i] == "E" {
                var j = i + 1
                if j < scalars.count, scalars[j] == "-" || scalars[j] == "+" { j += 1 }
                var expDigits = 0
                while j < scalars.count, isDigit(scalars[j]) { j += 1; expDigits += 1 }
                if expDigits > 0 { i = j }
            }
            var text = String.UnicodeScalarView()
            text.append(contentsOf: scalars[begin..<i])
            guard let value = Double(String(text)) else { return nil }
            index = i
            return value
        }

        /// An arc flag: a single `0` or `1` (it may be written without a separator after it).
        mutating func flag() -> Bool? {
            skipSeparators()
            guard let c = peek, c == "0" || c == "1" else { return nil }
            index += 1
            return c == "1"
        }
    }
}
