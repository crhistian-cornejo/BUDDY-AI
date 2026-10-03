import XCTest
@testable import Buddy

final class PixelImageTests: XCTestCase {
    func testMakesAnImageOfTheRightSizeOrRefusesBadInput() {
        let image = PixelImage.make(pixels: [0xFF00_0000, 0, 0, 0xFFFF_0000], size: 2)
        XCTAssertEqual(image?.width, 2)
        XCTAssertEqual(image?.height, 2)
        XCTAssertNil(PixelImage.make(pixels: [0, 0, 0], size: 2))
    }

    func testKeepsColorsAndTransparency() throws {
        XCTAssertEqual(try readPixel(0xFFFF_0000), [255, 0, 0, 255], "opaque red stays opaque red")
        XCTAssertEqual(try readPixel(0), [0, 0, 0, 0], "0 is transparent")
    }

    private func readPixel(_ argb: UInt32) throws -> [UInt8] {
        let image = try XCTUnwrap(PixelImage.make(pixels: [argb], size: 1))
        var bytes = [UInt8](repeating: 0, count: 4)
        let context = try XCTUnwrap(CGContext(
            data: &bytes, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.draw(image, in: CGRect(x: 0, y: 0, width: 1, height: 1))
        return bytes
    }

    func testCoreSpriteBecomesFrames() throws {
        let core = try BuddyCore(dataDir: NSTemporaryDirectory() + "buddy-tests-\(UUID().uuidString)")
        let sprite = try core.sprite(id: "buddy-base")
        let idle = try XCTUnwrap(sprite.states.first)
        XCTAssertEqual(idle.name, "idle")
        XCTAssertEqual(idle.frames.count, 8)
        for frame in idle.frames {
            XCTAssertNotNil(PixelImage.make(pixels: frame, size: Int(sprite.size)))
        }
        let names = Set(sprite.states.map(\.name))
        for state in ["blink", "look", "wave", "walk-left", "walk-right", "drag", "think", "work", "ask", "error", "done", "sleep"] {
            XCTAssertTrue(names.contains(state), "missing \(state)")
        }
    }

    func testDesignTokensShipInTheApp() {
        let tokens = DesignTokens.load()
        XCTAssertEqual(tokens.pet.scaleNormal, 2)
    }
}
