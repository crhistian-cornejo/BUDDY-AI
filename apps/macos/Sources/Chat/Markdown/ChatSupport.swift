// Pieces of MIKA (MIT, © MIKA contributors, revision d050bc5) that the ported chat views need:
// ChatSource (Providers/ChatModel.swift), Color(hex:) (Island/IslandRootView.swift) and Picks.webHost
// (Services/Picks.swift). `LiveMessage` is MIKA's ChatMessage reduced to what the bubble draws.
import AppKit
import SwiftUI

/// A page an answer used, shown as a small icon under the answer (never as a link in the text).
struct ChatSource: Codable, Equatable, Hashable, Sendable {
    var title: String
    var url: String

    var host: String { Self.host(of: url) ?? url }

    static func host(of url: String) -> String? {
        guard let host = URL(string: url)?.host?.lowercased(), !host.isEmpty else { return nil }
        return host.hasPrefix("www.") ? String(host.dropFirst(4)) : host
    }

    /// Only web addresses are ever sources: anything else (`javascript:`, `file:`…) is refused.
    static func make(title: String, url: String) -> ChatSource? {
        let address = url.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let parsed = URL(string: address), let scheme = parsed.scheme?.lowercased(), scheme == "http" || scheme == "https",
              let host = host(of: address) else { return nil }
        let name = title.trimmingCharacters(in: .whitespacesAndNewlines)
        return ChatSource(title: name.isEmpty ? host : name, url: address)
    }
}

/// One message as the chat panel draws it (the saved one comes from the core).
struct LiveMessage: Identifiable, Equatable, Sendable {
    var id = UUID()
    var role: String
    var content: String
    var isStreaming = false
    var sources: [ChatSource] = []
    /// What the agent is doing right now ("Buscando: clima Lima"). Never saved.
    var status: String?
    /// Who answered ("Buddy", "PARLEY") and with which provider ("claude", "codex").
    var author: String?
    var agentID = "buddy"
    var provider: String?
    /// Files attached to a user message (Buddy's copies).
    var files: [String] = []
    /// What the agent is doing right now, with its symbol. Never saved.
    var activity: ChatActivity?
    var failed = false
    /// The model that wrote an answer, in words («Opus 5.5 · esfuerzo alto»); shown in the mark's tooltip.
    var model: String?
}

/// A live line under the author: thinking, searching, reading, handing off.
struct ChatActivity: Equatable, Sendable {
    /// Empty: no symbol, only the shimmering text (thinking).
    var symbol: String
    var text: String

    static let thinking = ChatActivity(symbol: "", text: "Pensando…")

    static func tool(_ name: String, _ summary: String) -> ChatActivity {
        switch name {
        case "WebSearch":
            return ChatActivity(symbol: "magnifyingglass", text: summary.isEmpty ? "Buscando en la web…" : "Buscando: \(summary)")
        case "WebFetch":
            return ChatActivity(symbol: "doc.text.magnifyingglass", text: "Leyendo \(WebHost.of(summary) ?? "una página")…")
        case "Read":
            return ChatActivity(symbol: "doc.text.magnifyingglass", text: "Leyendo el archivo…")
        case "Cambio":
            return ChatActivity(symbol: "arrow.left.arrow.right", text: summary)
        case "Telegram":
            return ChatActivity(symbol: "paperplane", text: summary)
        case "Cuotas":
            return ChatActivity(symbol: "chart.line.uptrend.xyaxis", text: summary)
        default:
            return ChatActivity(symbol: "gearshape", text: "Trabajando…")
        }
    }

    static func handoff(to agent: String) -> ChatActivity {
        ChatActivity(symbol: "arrow.triangle.branch", text: "Buddy le pasa la tarea a \(agent)…")
    }
}

enum WebHost {
    static func of(_ url: String) -> String? {
        let lower = url.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let rest: Substring
        if lower.hasPrefix("https://") { rest = lower.dropFirst(8) }
        else if lower.hasPrefix("http://") { rest = lower.dropFirst(7) }
        else { return nil }
        let authority = rest.prefix { $0 != "/" && $0 != "?" && $0 != "#" }
        let afterAt = authority.split(separator: "@", omittingEmptySubsequences: false).last ?? ""
        let host = afterAt.split(separator: ":", omittingEmptySubsequences: false).first ?? ""
        let odd = host.unicodeScalars.contains {
            CharacterSet.whitespacesAndNewlines.contains($0) || $0.properties.generalCategory == .control
        }
        guard !host.isEmpty, !odd else { return nil }
        return host.hasPrefix("www.") ? String(host.dropFirst(4)) : String(host)
    }
}

extension Color {
    init(hex: String) {
        let h = hex.trimmingCharacters(in: CharacterSet(charactersIn: "#"))
        let val = UInt64(h, radix: 16) ?? 0
        self.init(red: Double((val >> 16) & 0xFF) / 255, green: Double((val >> 8) & 0xFF) / 255,
                  blue: Double(val & 0xFF) / 255)
    }
}

extension Color {
    /// A colour that follows the system appearance (light or dark).
    static func dynamic(light: String, dark: String) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            let isDark = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            return NSColor(Color(hex: isDark ? dark : light))
        })
    }
}


/// A brand mark as an image, to sit inside a line of text (inline workspace links).
@MainActor
enum BrandIcons {
    private static var cache: [String: NSImage] = [:]

    static func image(_ mark: BrandMark, size: CGFloat) -> NSImage {
        let key = "\(mark.rawValue)-\(Int(size * 10))"
        if let hit = cache[key] { return hit }
        let renderer = ImageRenderer(content: BrandMarkShape(mark: mark).fill(mark.color).frame(width: size, height: size))
        renderer.scale = NSScreen.main?.backingScaleFactor ?? 2
        let image = renderer.nsImage ?? NSImage(size: NSSize(width: size, height: size))
        cache[key] = image
        return image
    }
}
