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
