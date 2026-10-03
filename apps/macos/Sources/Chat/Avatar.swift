import AppKit
import Observation
import SwiftUI

/// The agents' faces (the avatar square the core gives with each sprite), cropped from idle frame 0 and cached per
/// agent. Buddy's comes from buddy-base until its own face is asked for (they are the same pixels by default).
@MainActor
enum Avatar {
    private(set) static var buddy: CGImage?
    private static var faces: [String: CGImage] = [:]
    private static var ids: [String: String] = [:]

    static func configure(sprite: Sprite) {
        buddy = face(of: sprite)
    }

    /// The avatar square of idle frame 0.
    static func face(of sprite: Sprite) -> CGImage? {
        guard let face = sprite.face,
              let idle = sprite.states.first(where: { $0.name == "idle" })?.frames.first,
              let image = PixelImage.make(pixels: idle, size: Int(sprite.size)) else { return nil }
        return image.cropping(to: CGRect(x: Int(face.x), y: Int(face.y), width: Int(face.size), height: Int(face.size)))
    }

    /// An agent's face, made once (until `invalidate`).
    static func face(agentId: String) -> CGImage? {
        if let cached = faces[agentId] { return cached }
        guard let core = AppServices.core, let sprite = try? core.agentSprite(agentId: agentId),
              let image = face(of: sprite) else { return agentId == "buddy" ? buddy : nil }
        faces[agentId] = image
        return image
    }

    /// The id of the agent with that name (the chat keeps names); Buddy when unknown.
    static func agentId(named name: String) -> String {
        if ids[name] == nil, let core = AppServices.core {
            ids = Dictionary(core.agents().map { ($0.name, $0.id) }, uniquingKeysWith: { a, _ in a })
        }
        return ids[name] ?? (name == "Buddy" ? "buddy" : name.lowercased())
    }

    /// Forget a face (or all) after Settings changes it; the views showing it draw again.
    static func invalidate(agentId: String? = nil) {
        if let agentId { faces[agentId] = nil } else { faces = [:]; ids = [:] }
        AvatarRevision.shared.value += 1
    }
}

/// Bumped when a face changes, so the views showing faces redraw.
@MainActor
@Observable
final class AvatarRevision {
    static let shared = AvatarRevision()
    var value = 0
}

/// Buddy's pixel face at a small size, crisp.
struct AvatarView: View {
    var size: CGFloat = 18

    var body: some View {
        AgentAvatarView(agentId: "buddy", size: size)
    }
}

/// An agent's pixel face (its own colour, accessory and eyes) at a small size, crisp.
struct AgentAvatarView: View {
    let agentId: String
    var size: CGFloat = 18

    var body: some View {
        let _ = AvatarRevision.shared.value
        if let image = Avatar.face(agentId: agentId) {
            Image(decorative: image, scale: 1)
                .interpolation(.none)
                .resizable()
                .frame(width: size, height: size)
        } else {
            Image(systemName: "person.crop.circle").frame(width: size, height: size)
        }
    }
}

/// The small mark of the service that wrote an answer, with its name as a tooltip.
struct ProviderMark: View {
    let provider: String?
    var size: CGFloat = 12
    var showsTooltip = true

    var body: some View {
        switch provider {
        case "claude":
            mark(.claude, "Escrito con Claude")
        case "codex":
            mark(.openai, "Escrito con Codex (ChatGPT)")
        case "antigravity", "gemini":
            if let image = GeminiMark.image {
                Image(nsImage: image).resizable().scaledToFit()
                    .frame(width: size, height: size)
                    .tip(showsTooltip ? "Escrito con Gemini" : "")
                    .accessibilityLabel("Escrito con Gemini")
            }
        default:
            EmptyView()
        }
    }

    private func mark(_ brand: BrandMark, _ help: String) -> some View {
        BrandMarkShape(mark: brand)
            .fill(brand == .openai ? Color.primary : brand.color)
            .frame(width: size, height: size)
            .tip(showsTooltip ? help : "")
            .accessibilityLabel(help)
    }
}

@MainActor
private enum GeminiMark {
    static let image: NSImage? = Bundle.main.url(forResource: "gemini", withExtension: "svg").flatMap { NSImage(contentsOf: $0) }
}
