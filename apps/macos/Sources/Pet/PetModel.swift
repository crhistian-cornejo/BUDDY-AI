import AppKit
import Observation

/// Plays the mascot's states (frames from the core). Only the current frame is published; between plans nothing runs.
@MainActor
@Observable
final class PetModel {
    private(set) var image: CGImage?
    private(set) var state = "idle"
    let size: Int

    @ObservationIgnored private var states: [String: (fps: Double, frames: [CGImage])] = [:]
    @ObservationIgnored private let maxFps: Double
    @ObservationIgnored private var playback = 0

    init(sprite: Sprite, maxFps: Double) {
        size = Int(sprite.size)
        self.maxFps = maxFps
        for s in sprite.states {
            let frames = s.frames.compactMap { PixelImage.make(pixels: $0, size: Int(sprite.size)) }
            if !frames.isEmpty { states[s.name] = (min(Double(s.fps), maxFps), frames) }
        }
        show("idle")
    }

    func has(_ name: String) -> Bool { states[name] != nil }

    func fps(_ name: String) -> Double { states[name]?.fps ?? 1 }

    func show(_ name: String, frame: Int = 0) {
        guard let s = states[name] ?? states["idle"] else { return }
        state = name
        image = s.frames[frame % s.frames.count]
    }

    /// Loops `name` for `duration` seconds (at least one pass), calling `step` once per frame, then holds the still
    /// frame of `rest` (idle, or sit while Buddy is seated). Returns false when something else interrupted it.
    @discardableResult
    func play(_ name: String, duration: TimeInterval, rest: String = "idle",
              step: ((TimeInterval) -> Void)? = nil) async -> Bool {
        guard let s = states[name] else { return true }
        playback += 1
        let token = playback
        if NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            show(name)
            try? await Task.sleep(for: .seconds(duration))
            guard token == playback && !Task.isCancelled else { return false }
            show(rest)
            return true
        }
        let interval = 1 / s.fps
        let count = max(s.frames.count, Int((duration / interval).rounded()))
        for i in 0..<count {
            if Task.isCancelled || token != playback { return false }
            show(name, frame: i)
            step?(interval)
            try? await Task.sleep(for: .seconds(interval))
        }
        guard !Task.isCancelled && token == playback else { return false }
        show(rest)
        return true
    }

    /// Loops `name` until the task is cancelled (agent states, being dragged).
    func loop(_ name: String) async {
        guard let s = states[name] else { return }
        playback += 1
        let token = playback
        if NSWorkspace.shared.accessibilityDisplayShouldReduceMotion { show(name); return }
        var i = 0
        while !Task.isCancelled && token == playback {
            show(name, frame: i)
            i += 1
            if s.frames.count == 1 { break }
            try? await Task.sleep(for: .seconds(1 / s.fps))
        }
    }
}
