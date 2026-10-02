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
    /// Who answered: "Buddy · Claude", "PARLEY · Claude".
    var author: String?
    var failed = false
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

extension View {
    /// MIKA's tooltip; here the system one.
    func tip(_ text: String) -> some View { help(text) }
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
