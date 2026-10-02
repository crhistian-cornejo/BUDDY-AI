import CoreGraphics
import Foundation

/// Turns a core frame (`0xAARRGGBB` per pixel, row by row) into a CGImage. Pixels are fully opaque or fully
/// transparent, so premultiplied and straight alpha are the same bytes.
enum PixelImage {
    static func make(pixels: [UInt32], size: Int) -> CGImage? {
        guard size > 0, pixels.count == size * size else { return nil }
        let data = pixels.withUnsafeBufferPointer { Data(buffer: $0) }
        guard let provider = CGDataProvider(data: data as CFData) else { return nil }
        // 32-bit little-endian words of ARGB = bytes B, G, R, A in memory.
        let info = CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedFirst.rawValue)
            .union(.byteOrder32Little)
        return CGImage(width: size, height: size, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: size * 4,
                       space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: info, provider: provider,
                       decode: nil, shouldInterpolate: false, intent: .defaultIntent)
    }
}
