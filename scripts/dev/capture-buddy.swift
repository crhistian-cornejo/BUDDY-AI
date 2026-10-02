// Development only: captures Buddy's own windows (never the rest of the screen) into one PNG per window.
// Usage: swift scripts/dev/capture-buddy.swift <output-folder>
import CoreGraphics
import Foundation

let folder = CommandLine.arguments.dropFirst().first ?? "."
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
var n = 0
for info in list where (info[kCGWindowOwnerName as String] as? String) == "Buddy" {
    guard let id = info[kCGWindowNumber as String] as? Int,
          let bounds = info[kCGWindowBounds as String] as? [String: CGFloat], (bounds["Width"] ?? 0) > 60 else { continue }
    let out = "\(folder)/buddy-\(n).png"
    let task = Process()
    task.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
    task.arguments = ["-x", "-o", "-l", String(id), out]
    try? task.run()
    task.waitUntilExit()
    print(out, Int(bounds["Width"] ?? 0), "x", Int(bounds["Height"] ?? 0))
    n += 1
}
