import SwiftUI

/// The mascot with no smoothing: every logical pixel is a crisp square (whole device pixels on Retina, also at ×1.5).
struct PetView: View {
    let model: PetModel
    let scale: Double

    var body: some View {
        let side = CGFloat(Double(model.size) * scale)
        Group {
            if let image = model.image {
                Image(decorative: image, scale: 1)
                    .interpolation(.none)
                    .resizable()
            } else {
                Color.clear
            }
        }
        .frame(width: side, height: side)
        .accessibilityLabel("Buddy")
    }
}

/// A short line from Buddy above (or below) the mascot.
struct BubbleView: View {
    let text: String
    let tokens: DesignTokens

    var body: some View {
        Text(text)
            .font(.system(size: tokens.font.sizeBody, weight: .medium))
            .foregroundStyle(Color(nsColor: NSColor(hex: tokens.color.text)))
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(
                RoundedRectangle(cornerRadius: tokens.radius.bubble, style: .continuous)
                    .fill(Color(nsColor: NSColor(hex: tokens.color.surface)))
                    .overlay(
                        RoundedRectangle(cornerRadius: tokens.radius.bubble, style: .continuous)
                            .strokeBorder(Color(nsColor: NSColor(hex: tokens.color.stroke)), lineWidth: 1)
                    )
            )
            .fixedSize()
            .padding(6)
    }
}
