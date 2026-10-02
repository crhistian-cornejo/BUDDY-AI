import AppKit

/// assets/design-tokens.json, shared with the Windows app (copied into the bundle as a resource).
struct DesignTokens: Decodable, Sendable {
    struct Colors: Decodable, Sendable {
        var surface: String
        var surfaceRaised: String
        var stroke: String
        var text: String
        var textMuted: String
        var accent: String
        var success: String
        var danger: String
    }

    struct Radius: Decodable, Sendable {
        var control: Double
        var bubble: Double
        var card: Double
        var island: Double
    }

    struct Font: Decodable, Sendable {
        var sizeSmall: Double
        var sizeBody: Double
        var sizeTitle: Double
    }

    struct Motion: Decodable, Sendable {
        var springResponse: Double
        var springDamping: Double
        var fadeOut: Double
        var breathEveryMin: Double
        var breathEveryMax: Double
        var breathDuration: Double
        var maxFps: Double
        var helloSeconds: Double
    }

    struct Pet: Decodable, Sendable {
        var scaleSmall: Double
        var scaleNormal: Double
        var scaleLarge: Double
        var margin: Double
    }

    var color: Colors
    var radius: Radius
    var font: Font
    var motion: Motion
    var pet: Pet

    static func load(bundle: Bundle = .main) -> DesignTokens {
        guard let url = bundle.url(forResource: "design-tokens", withExtension: "json"),
              let data = try? Data(contentsOf: url),
              let tokens = try? JSONDecoder().decode(DesignTokens.self, from: data)
        else {
            fatalError("design-tokens.json falta en el paquete de la app o no es válido")
        }
        return tokens
    }
}

extension NSColor {
    /// `#RRGGBB` → sRGB color (black when malformed).
    convenience init(hex: String) {
        let value = UInt32(hex.trimmingCharacters(in: CharacterSet(charactersIn: "#")), radix: 16) ?? 0
        self.init(srgbRed: CGFloat((value >> 16) & 0xFF) / 255,
                  green: CGFloat((value >> 8) & 0xFF) / 255,
                  blue: CGFloat(value & 0xFF) / 255,
                  alpha: 1)
    }
}

import SwiftUI

extension Color {
    /// The palette's second colour (deep indigo; a lighter shade in dark mode). The first, mint, is the app's accent.
    static let buddyIndigo = Color("BuddyIndigo")
}
