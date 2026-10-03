import AppKit
import ScreenCaptureKit
import UniformTypeIdentifiers

/// One screenshot for an agent, after the user's «Permitir» on the card: the screen under the pointer, without
/// Buddy's own windows. macOS asks once for Screen Recording; without it the capture fails and the agent says so.
enum ScreenCapture {
    static func capture(to path: String) async -> Bool {
        do {
            let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
            let mouse = NSEvent.mouseLocation
            let screen = NSScreen.screens.first { $0.frame.contains(mouse) } ?? NSScreen.main
            let number = screen?.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? CGDirectDisplayID
            guard let display = content.displays.first(where: { $0.displayID == number }) ?? content.displays.first else { return false }
            let buddy = content.applications.filter { $0.processID == ProcessInfo.processInfo.processIdentifier }
            let filter = SCContentFilter(display: display, excludingApplications: buddy, exceptingWindows: [])
            let config = SCStreamConfiguration()
            let scale = screen?.backingScaleFactor ?? 2
            config.width = Int(CGFloat(display.width) * scale)
            config.height = Int(CGFloat(display.height) * scale)
            config.showsCursor = false
            let image = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
            return write(image, to: URL(fileURLWithPath: path))
        } catch {
            NSLog("Buddy: captura fallida: \(error.localizedDescription)")
            return false
        }
    }

    private static func write(_ image: CGImage, to url: URL) -> Bool {
        guard let destination = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil) else { return false }
        CGImageDestinationAddImage(destination, image, nil)
        return CGImageDestinationFinalize(destination)
    }
}
