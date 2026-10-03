@preconcurrency import AVFoundation
import Observation
import Speech

/// The microphone button of the composer: what is said is written into the draft while it is on, with the Speech
/// framework of macOS 27 (`CaptureInputSequenceProvider` → `SpeechAnalyzer` + `DictationTranscriber`, the system's
/// dictation model, which writes commas, full stops and question and exclamation marks by itself), on this Mac.
/// Nothing listens while the button is off.
@MainActor
@Observable
final class Dictation {
    private(set) var recording = false
    /// Starting up (permissions, the model): the button shows it is on its way.
    private(set) var preparing = false
    /// How loud the voice is, 0…1 (the button's halo).
    private(set) var level = 0.0
    private(set) var problem: String?

    @ObservationIgnored private var session: Task<Void, Never>?
    @ObservationIgnored private var teardown: (() async -> Void)?
    @ObservationIgnored private var output: AVCaptureAudioDataOutput?
    @ObservationIgnored private var meter: Timer?
    /// Bumped whenever a dictation ends: results of an older one never touch the draft again.
    @ObservationIgnored private var take = 0

    func toggle(into chat: ChatController) {
        if recording || preparing { stop() } else { start(into: chat) }
    }

    /// `discard`: the message was sent, so whatever the recogniser still has to say is dropped (a plain stop keeps
    /// the last words, which arrive a moment later).
    func stop(discard: Bool = false) {
        if discard { take += 1 }
        let end = teardown
        teardown = nil
        meter?.invalidate()
        meter = nil
        output = nil
        recording = false
        preparing = false
        level = 0
        let running = session
        session = nil
        // The last words are finalised before the session goes away.
        Task {
            await end?()
            running?.cancel()
        }
    }

    private func start(into chat: ChatController) {
        problem = nil
        preparing = true
        session = Task { [weak self, weak chat] in
            guard #available(macOS 27, *) else {
                self?.fail("El dictado necesita macOS 27.")
                return
            }
            guard await Self.allowed() else {
                self?.fail("Permite Micrófono y Reconocimiento de voz para Buddy en Ajustes del Sistema › Privacidad y seguridad.")
                return
            }
            do {
                try await self?.listen(into: chat)
                Self.log("la sesión terminó sin error")
                if self?.recording == true { self?.stop() }
            } catch is CancellationError {
            } catch {
                Self.log("error: \(error)")
                if self?.recording == true || self?.preparing == true { self?.fail("El dictado se detuvo: \(error.localizedDescription)") }
            }
        }
    }

    private func fail(_ text: String) {
        stop()
        problem = text
    }

    /// The system's permission prompts. Their callbacks arrive on other threads: nothing here touches the main actor.
    private nonisolated static func allowed() async -> Bool {
        guard await AVCaptureDevice.requestAccess(for: .audio) else { return false }
        return await withCheckedContinuation { done in
            SFSpeechRecognizer.requestAuthorization { status in done.resume(returning: status == .authorized) }
        }
    }

    @available(macOS 27, *)
    private func listen(into chat: ChatController?) async throws {
        // Spanish first (the app's language); otherwise what the Mac is set to.
        var locale: Locale?
        for id in [Locale.current.identifier, "es_MX", "es_US", "es_ES"] where locale == nil && id.hasPrefix("es") {
            locale = await DictationTranscriber.supportedLocale(equivalentTo: Locale(identifier: id))
        }
        if locale == nil { locale = await DictationTranscriber.supportedLocale(equivalentTo: Locale.current) }
        guard let locale else { throw Problem("este Mac no tiene transcripción para tu idioma") }
        // `.frequentFinalization` is left out: with it, live capture gives no results at all on macOS 27.0.
        let transcriber = DictationTranscriber(locale: locale, contentHints: [], transcriptionOptions: [.punctuation], reportingOptions: [.volatileResults], attributeOptions: [])
        let modules: [any SpeechModule] = [transcriber]
        if let request = try await AssetInventory.assetInstallationRequest(supporting: modules) {
            problem = "Descargando la voz para el dictado…"
            try await request.downloadAndInstall()
            problem = nil
        }
        guard let microphone = AVCaptureDevice.default(for: .audio) else { throw Problem("no hay micrófono") }
        let capture = try await CaptureInputSequenceProvider.providerWithSession(from: microphone, compatibleWith: modules)
        let analyzer = SpeechAnalyzer(modules: modules)
        // Names the recogniser would spell by ear (Buddy, Niko, Notion, Yape…) and the files attached right now.
        let context = AnalysisContext()
        let files = (chat?.attachments ?? []).map { $0.deletingPathExtension().lastPathComponent }
        context.contextualStrings[.general] = (AppServices.core?.voiceVocabulary() ?? []) + files
        try? await analyzer.setContext(context)
        try await analyzer.start(inputSequence: capture.analyzerInputs)
        let captureSession = capture.captureSession
        guard preparing else {
            await analyzer.cancelAndFinishNow()
            return
        }
        output = capture.captureAudioDataOutput
        teardown = {
            await Self.halt(captureSession)
            try? await analyzer.finalize(through: nil)
            await analyzer.cancelAndFinishNow()
        }
        await Self.run(captureSession)
        preparing = false
        recording = true
        startMeter()
        Self.log("dictando (\(locale.identifier))")

        // What was already typed stays; the dictation goes after it.
        let before = chat?.draft ?? ""
        var committed = ""
        take += 1
        let current = take
        for try await result in transcriber.results {
            guard current == take else { break }
            let piece = String(result.text.characters)
            #if DEBUG
            if ProcessInfo.processInfo.environment["BUDDY_DEBUG_VOICE_LOG"] != nil { Self.log("\(result.isFinal ? "final" : "parcial") «\(piece)»") }
            #endif
            let said = Self.join(committed, piece)
            if result.isFinal { committed = said }
            // Spanish as it is written: «¿», «¡» and capitals (the core's `voice::tidy`).
            chat?.draft = Self.join(before, AppServices.core?.voiceTidy(text: said) ?? said)
        }
    }

    private static func join(_ a: String, _ b: String) -> String {
        let (a, b) = (a.trimmingCharacters(in: .whitespaces), b.trimmingCharacters(in: .whitespaces))
        return a.isEmpty ? b : b.isEmpty ? a : a + " " + b
    }

    /// `startRunning` and `stopRunning` block: off the main actor.
    private nonisolated static func run(_ session: AVCaptureSession) async {
        if !session.isRunning { session.startRunning() }
    }

    private nonisolated static func halt(_ session: AVCaptureSession) async {
        if session.isRunning { session.stopRunning() }
    }

    private func startMeter() {
        meter?.invalidate()
        let timer = Timer(timeInterval: 1.0 / 12, repeats: true) { [weak self] _ in
            Task { @MainActor [weak self] in self?.readLevel() }
        }
        RunLoop.main.add(timer, forMode: .common)
        meter = timer
    }

    private func readLevel() {
        guard recording, let channel = output?.connections.first?.audioChannels.first else { return }
        // Speech sits around -45…-10 dB. Rises at once, falls slowly.
        let value = Double(min(max((channel.averagePowerLevel + 50) / 40, 0), 1))
        level = max(value, level * 0.8)
    }

    /// Diagnostics next to the core's log (`voz.log`): sessions and their errors; never what was said, except with
    /// BUDDY_DEBUG_VOICE_LOG in a debug build.
    private nonisolated static func log(_ text: String) {
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("Buddy")
        let line = "\(ISO8601DateFormatter().string(from: Date())) \(text)\n"
        let url = dir.appendingPathComponent("voz.log")
        if let handle = try? FileHandle(forWritingTo: url) {
            defer { try? handle.close() }
            _ = try? handle.seekToEnd()
            try? handle.write(contentsOf: Data(line.utf8))
        } else {
            try? Data(line.utf8).write(to: url)
        }
    }

    private struct Problem: LocalizedError {
        let errorDescription: String?
        init(_ text: String) { errorDescription = text }
    }
}
