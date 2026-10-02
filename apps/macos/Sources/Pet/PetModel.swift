import AppKit
import Observation

/// Which frame of the mascot is on screen. Idle is one still frame; every 6–10 s Buddy breathes for ~2 s at the
/// state's fps and stops. Nothing runs between breaths (the task sleeps), and with "reduce motion" it never moves.
@MainActor
@Observable
final class PetModel {
    private(set) var image: CGImage?
    let size: Int

    @ObservationIgnored private let idleFrames: [CGImage]
    @ObservationIgnored private let idleFps: Double
    @ObservationIgnored private let motion: DesignTokens.Motion
    @ObservationIgnored private var task: Task<Void, Never>?

    init(sprite: Sprite, motion: DesignTokens.Motion) {
        size = Int(sprite.size)
        let idle = sprite.states.first { $0.name == "idle" } ?? sprite.states.first
        idleFrames = (idle?.frames ?? []).compactMap { PixelImage.make(pixels: $0, size: Int(sprite.size)) }
        idleFps = min(Double(idle?.fps ?? 2), motion.maxFps)
        self.motion = motion
        image = idleFrames.first
    }

    func start() {
        guard task == nil else { return }
        task = Task { [weak self] in await self?.idleLoop() }
    }

    func stop() {
        task?.cancel()
        task = nil
    }

    private func idleLoop() async {
        while !Task.isCancelled {
            let wait = Double.random(in: motion.breathEveryMin...motion.breathEveryMax)
            try? await Task.sleep(for: .seconds(wait))
            guard idleFrames.count > 1, idleFps > 0,
                  !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion else { continue }
            let steps = max(1, Int(motion.breathDuration * idleFps))
            for step in 1...steps where !Task.isCancelled {
                image = idleFrames[step % idleFrames.count]
                try? await Task.sleep(for: .seconds(1 / idleFps))
            }
            image = idleFrames.first
        }
    }
}
