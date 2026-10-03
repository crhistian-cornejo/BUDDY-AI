import SwiftUI

/// Buddy, painted from the core's pixels (the same sprite the desktop draws, with the look chosen on that machine).
/// It plays the frames of `state` at the sprite's own pace; with «Reducir movimiento» it holds the first frame.
struct PixelSpriteView: View {
    let sprite: Sprite?
    var state = "idle"
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        if let sprite, let frames = frames(of: sprite), !frames.isEmpty {
            let fps = Double(max(1, sprite.states.first { $0.name == state }?.fps ?? 4))
            TimelineView(.animation(minimumInterval: 1 / fps, paused: reduceMotion || frames.count < 2)) { context in
                let index = reduceMotion ? 0 : Int(context.date.timeIntervalSinceReferenceDate * fps) % frames.count
                Image(decorative: frames[index], scale: 1)
                    .interpolation(.none)
                    .resizable()
                    .aspectRatio(1, contentMode: .fit)
            }
            .accessibilityElement()
            .accessibilityLabel("Buddy")
        } else {
            // Until the machine sends the sprite: a quiet stand-in of the same size.
            RoundedRectangle(cornerRadius: 24, style: .continuous)
                .fill(.tint.opacity(0.15))
                .overlay { Image(systemName: "leaf.fill").font(.largeTitle).foregroundStyle(.tint) }
                .aspectRatio(1, contentMode: .fit)
                .accessibilityHidden(true)
        }
    }

    private func frames(of sprite: Sprite) -> [CGImage]? {
        let wanted = sprite.states.first { $0.name == state } ?? sprite.states.first { $0.name == "idle" } ?? sprite.states.first
        return wanted?.frames.compactMap { Self.image(pixels: $0, size: sprite.size) }
    }

    /// A frame (`0xAARRGGBB` per pixel, row by row) as an image. Pixels are opaque or transparent, so premultiplied
    /// and straight alpha are the same bytes.
    static func image(pixels: [UInt32], size: Int) -> CGImage? {
        guard size > 0, pixels.count == size * size else { return nil }
        let data = pixels.withUnsafeBufferPointer { Data(buffer: $0) }
        guard let provider = CGDataProvider(data: data as CFData), let space = CGColorSpace(name: CGColorSpace.sRGB) else { return nil }
        let info = CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedFirst.rawValue).union(.byteOrder32Little)
        return CGImage(width: size, height: size, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: size * 4, space: space, bitmapInfo: info,
                       provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
    }
}
