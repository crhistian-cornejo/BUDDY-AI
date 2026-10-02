import AppKit
import SwiftUI

/// Buddy's face (the avatar square the core gives with the character), cropped once from idle frame 0.
@MainActor
enum Avatar {
    private(set) static var buddy: CGImage?

    static func configure(sprite: Sprite) {
        guard let face = sprite.face,
              let idle = sprite.states.first(where: { $0.name == "idle" })?.frames.first,
              let image = PixelImage.make(pixels: idle, size: Int(sprite.size)) else { return }
        buddy = image.cropping(to: CGRect(x: Int(face.x), y: Int(face.y), width: Int(face.size), height: Int(face.size)))
    }
}

/// Buddy's pixel face at a small size, crisp.
struct AvatarView: View {
    var size: CGFloat = 18

    var body: some View {
        if let image = Avatar.buddy {
            Image(decorative: image, scale: 1)
                .interpolation(.none)
                .resizable()
                .frame(width: size, height: size)
        } else {
            Image(systemName: "sparkle").frame(width: size, height: size)
        }
    }
}

/// The small mark of the service that wrote an answer, with its name as a tooltip.
struct ProviderMark: View {
    let provider: String?
    var size: CGFloat = 12

    var body: some View {
        switch provider {
        case "claude":
            mark(.claude, "Escrito con Claude")
        case "codex":
            mark(.openai, "Escrito con Codex (ChatGPT)")
        case "antigravity":
            Image(systemName: "sparkles")
                .font(.system(size: size - 1, weight: .semibold))
                .foregroundStyle(.blue)
                .help("Escrito con Gemini")
        default:
            EmptyView()
        }
    }

    private func mark(_ brand: BrandMark, _ help: String) -> some View {
        BrandMarkShape(mark: brand)
            .fill(brand == .openai ? Color.primary : brand.color)
            .frame(width: size, height: size)
            .help(help)
            .accessibilityLabel(help)
    }
}
