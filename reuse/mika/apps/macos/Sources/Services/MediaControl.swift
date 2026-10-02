import Foundation
import AppKit

// Media control: what Spotify or Apple Music is playing, and its previous / play-pause / next buttons, in a strip under
// the overview cards. Twin of apps/windows/src/integrations/media.ts + services/media.rs (same contract and rules:
// docs/SPEC.md, "Media control").
//
// How it works on the Mac: MediaRemote (the system's Now Playing) is a private API that third-party apps cannot use
// since macOS 15.4, so MIKA asks the two players directly with AppleScript (`/usr/bin/osascript`, run off the main
// thread and killed after 2.5 s). The first time, macOS asks the user to allow MIKA to control Spotify / Music
// (NSAppleEventsUsageDescription). Nothing leaves the Mac. A browser playing YouTube is NOT covered (the browsers have
// no scripting interface for it), Spotify provides a cover URL; Music provides its local artwork bytes.
//
// Volume: Spotify and Music both have `sound volume` (0-100) in their AppleScript dictionary. The strip's speaker
// button and slider drive THAT property of the shown player and nothing else: the Mac's output volume is never touched.
// Mute is "volume 0": the level before it is remembered (per player) and restored on unmute; unmuting from 0 goes to
// 30 %. The level is read in the same call as the track, so there is no extra Apple Event and no extra polling.
//
// Cost: `MediaWatcher` polls every second only while the island is open (or the pet's ring / music panel is) and the
// setting is on; the poll first asks NSWorkspace whether a player is running (a player that is not running is never
// touched, so MIKA never launches it).

enum MediaStatus: String, Sendable { case playing, paused, stopped }

/// The three buttons. The raw values are the Windows command's (`media_control`); anything else is refused.
enum MediaAction: String, CaseIterable, Sendable {
    case playPause = "play_pause"
    case next
    case previous

    /// Strict: an unknown action is nil, not a guess.
    static func validate(_ raw: String) -> MediaAction? { MediaAction(rawValue: raw) }

    /// The AppleScript command (the same words in Spotify and in Music).
    var command: String {
        switch self {
        case .playPause: return "playpause"
        case .next: return "next track"
        case .previous: return "previous track"
        }
    }
}

/// What is playing. Same fields as `NowPlaying` on Windows, minus the cover.
struct NowPlaying: Equatable, Sendable {
    var app: String
    var bundleID: String
    var title: String
    var artist: String
    var status: MediaStatus
    var positionMs: Int?
    var durationMs: Int?
    /// The player's own `sound volume`, 0...100; nil when it did not say (the speaker and the slider are then disabled).
    var volumePercent: Int? = nil
    var album: String = ""
    var trackID: String = ""
    var artworkURL: String? = nil
    var artwork: Data? = nil
    var artworkKey: String { [bundleID, trackID, title, artist, album, artworkURL ?? ""].joined(separator: "\u{1F}") }
    var canPlayPause = true
    var canNext = true
    var canPrevious = true

    /// The strip shows while something plays or is paused, not when it is stopped.
    var isVisible: Bool { status == .playing || status == .paused }

    /// Second line of the strip: "Artist · Spotify", or just the app when the artist is unknown.
    var subLine: String { [artist, app].filter { !$0.isEmpty }.joined(separator: " · ") }

    /// The first line: the title, or the app when the player gave none.
    var headline: String { title.isEmpty ? app : title }

    /// The speaker shows "off" at volume 0 (that is what muted means here).
    var isSilent: Bool { volumePercent.map { $0 <= 0 } ?? false }
}

/// The two players MIKA can talk to.
enum MediaPlayer: CaseIterable, Sendable {
    case spotify, music

    var bundleID: String {
        switch self {
        case .spotify: return "com.spotify.client"
        case .music: return "com.apple.Music"
        }
    }

    var appName: String {
        switch self {
        case .spotify: return "Spotify"
        case .music: return "Music"
        }
    }

    /// `duration of current track` is in milliseconds in Spotify and in seconds in Music.
    var durationUnitMs: Int {
        switch self {
        case .spotify: return 1
        case .music: return 1000
        }
    }

    init?(bundleID: String) {
        guard let player = Self.allCases.first(where: { $0.bundleID == bundleID }) else { return nil }
        self = player
    }
}

enum MediaControl {
    /// ASCII "record separator" between the fields the script returns: a track title can hold any printable character.
    static let separator = "\u{1E}"
    static let maxText = 200
    static let timeout: TimeInterval = 2.5

    // MARK: Pure helpers (tested)

    /// The island only listens while it is open and the setting is on (and MIKA is not paused). The pet's ring and music
    /// panel (`petOpen`) count as "open" too: they are the pet-mode way to the strip.
    static func wantWatch(enabled: Bool, expanded: Bool, paused: Bool = false, petOpen: Bool = false) -> Bool {
        enabled && (expanded || petOpen) && !paused
    }

    // MARK: Volume (pure helpers, tested)

    /// A level (a slider position, a script value) as 0...1; nil for NaN and infinities, which are never sent anywhere.
    static func clampLevel(_ value: Double) -> Double? {
        value.isFinite ? min(1, max(0, value)) : nil
    }

    /// Percent (0...100) of a level in 0...1, rounded; nil for NaN and infinities.
    static func percent(fromLevel level: Double) -> Int? {
        clampLevel(level).map { Int(($0 * 100).rounded()) }
    }

    /// Level in 0...1 of a percent, clamped.
    static func level(fromPercent percent: Int) -> Double {
        Double(clampPercent(percent)) / 100
    }

    static func clampPercent(_ percent: Int) -> Int { min(100, max(0, percent)) }

    /// The player's answer to `sound volume` ("42", "42.0"): a percent in 0...100, out-of-range values clamped; nil when
    /// the text is not a finite number.
    static func parseVolume(_ output: String) -> Int? {
        let text = output.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, let value = Double(text), value.isFinite else { return nil }
        return Int(min(100, max(0, value)).rounded())
    }

    /// Where unmuting from 0 goes when there is no remembered level.
    static let defaultUnmutePercent = 30

    /// What the speaker button does at `currentPercent`: audible → remember it and go to 0; silent → go back to the
    /// remembered level, or to 30 % when there is none (or it was 0).
    static func speakerClick(currentPercent: Int, remembered: Int?) -> (newPercent: Int, remember: Int?) {
        let current = clampPercent(currentPercent)
        if current > 0 { return (0, current) }
        if let remembered, remembered > 0 { return (clampPercent(remembered), nil) }
        return (defaultUnmutePercent, nil)
    }

    /// Sets the player's own volume (never the system's). The number is clamped here, whatever the caller passes.
    static func volumeScript(percent: Int, for player: MediaPlayer) -> String {
        "tell application id \"\(player.bundleID)\" to set sound volume to \(clampPercent(percent))"
    }

    /// Reads the player's state in one call: "stopped", or state, title, artist, position (s) and duration separated by
    /// `separator`. Written against the bundle id (`application id`), so it never depends on the app's name.
    static func readScript(for player: MediaPlayer) -> String {
        let artwork = player == .spotify ? "set coverURL to artwork url of t" : ""
        let identity = player == .spotify ? "id of t" : "persistent ID of t"
        return """
        tell application id "\(player.bundleID)"
            set playerState to player state as text
            if playerState is "stopped" then return "stopped"
            set sep to (ASCII character 30)
            set t to current track
            set vol to ""
            try
                set vol to (sound volume as integer) as text
            end try
            set recordName to ""
            set trackID to ""
            set coverURL to ""
            try
                set recordName to album of t
                set trackID to \(identity)
                \(artwork)
            end try
            return playerState & sep & (name of t) & sep & (artist of t) & sep & ((player position as integer) as text) & sep & ((duration of t as integer) as text) & sep & vol & sep & recordName & sep & trackID & sep & coverURL
        end tell
        """
    }

    /// One button press.
    static func controlScript(_ action: MediaAction, for player: MediaPlayer) -> String {
        "tell application id \"\(player.bundleID)\" to \(action.command)"
    }

    /// Turns the script's output into a track; nil when nothing plays, is paused, or the text is not what was asked for.
    static func parse(_ output: String, player: MediaPlayer) -> NowPlaying? {
        let parts = output.trimmingCharacters(in: .whitespacesAndNewlines).components(separatedBy: separator)
        guard parts.count >= 3 else { return nil }
        let status: MediaStatus
        switch parts[0] {
        case "playing": status = .playing
        case "paused": status = .paused
        default: return nil
        }
        let title = clean(parts[1]), artist = clean(parts[2])
        guard !title.isEmpty || !artist.isEmpty else { return nil }
        func number(_ index: Int, scale: Int) -> Int? {
            guard parts.count > index, let value = Int(parts[index].trimmingCharacters(in: .whitespaces)), value >= 0
            else { return nil }
            let (scaled, overflow) = value.multipliedReportingOverflow(by: scale)
            return overflow ? nil : scaled
        }
        let volume = parts.count > 5 ? parseVolume(parts[5]) : nil
        return NowPlaying(app: player.appName, bundleID: player.bundleID, title: title, artist: artist, status: status,
                          positionMs: number(3, scale: 1000), durationMs: number(4, scale: player.durationUnitMs),
                          volumePercent: volume,
                          album: parts.count > 6 ? clean(parts[6]) : "",
                          trackID: parts.count > 7 ? clean(parts[7]) : "",
                          artworkURL: parts.count > 8 ? artworkURL(parts[8])?.absoluteString : nil)
    }

    /// Spotify supplies the artwork URL; do not accept local files or non-HTTPS schemes.
    static func artworkURL(_ text: String) -> URL? {
        guard let url = URL(string: text.trimmingCharacters(in: .whitespacesAndNewlines)),
              url.scheme == "https", let host = url.host,
              host == "scdn.co" || host.hasSuffix(".scdn.co"), url.user == nil, url.password == nil else { return nil }
        return url
    }

    static func artworkData(for track: NowPlaying) async -> Data? {
        if let raw = track.artworkURL, let url = artworkURL(raw) {
            do {
                var request = URLRequest(url: url)
                request.timeoutInterval = 6
                let (bytes, response) = try await URLSession.shared.bytes(for: request)
                guard let http = response as? HTTPURLResponse, http.statusCode == 200,
                      response.expectedContentLength <= 8_000_000 else { return nil }
                var data = Data()
                for try await byte in bytes {
                    if Task.isCancelled || data.count >= 8_000_000 { return nil }
                    data.append(byte)
                }
                return data
            } catch { return nil }
        }
        guard track.bundleID == MediaPlayer.music.bundleID, track.trackID.count == 16,
              track.trackID.allSatisfy({ $0.isHexDigit }) else { return nil }
        // Music exposes the original artwork bytes through its local scripting dictionary.
        return await Task.detached(priority: .utility) {
            let file = FileManager.default.temporaryDirectory.appendingPathComponent("mika-cover-\(UUID().uuidString).img")
            defer { try? FileManager.default.removeItem(at: file) }
            let script = """
            tell application id "com.apple.Music"
                if not running then return ""
                if (persistent ID of current track) is not "\(track.trackID)" then return ""
                set coverData to raw data of artwork 1 of current track
            end tell
            set destination to POSIX file "\(file.path)"
            set handle to open for access destination with write permission
            try
                set eof handle to 0
                write coverData to handle
                close access handle
            on error
                try
                    close access handle
                end try
            end try
            """
            guard runScript(script) != nil,
                  let size = try? file.resourceValues(forKeys: [.fileSizeKey]).fileSize, size <= 8_000_000 else { return nil }
            return try? Data(contentsOf: file)
        }.value
    }

    /// Which of the readings is shown: one that is playing, otherwise one that is paused (the first of each).
    static func choose(_ readings: [NowPlaying]) -> NowPlaying? {
        readings.first { $0.status == .playing } ?? readings.first { $0.status == .paused }
    }

    /// Single line, control characters out, cut to `maxText`.
    static func clean(_ text: String) -> String {
        let scalars = text.unicodeScalars.map { $0.properties.generalCategory == .control ? " " : String($0) }
        let line = scalars.joined().split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
        return String(line.prefix(maxText))
    }

    // MARK: Talking to the players

    /// The players that are running right now. Cheap (no Apple Event): it is what keeps MIKA from launching Spotify.
    static func runningPlayers() -> [MediaPlayer] {
        let running = Set(NSWorkspace.shared.runningApplications.compactMap(\.bundleIdentifier))
        return MediaPlayer.allCases.filter { running.contains($0.bundleID) }
    }

    /// Asks each given player (blocking, up to `timeout` each: call it off the main thread).
    static func read(_ players: [MediaPlayer]) -> NowPlaying? {
        choose(players.compactMap { player in
            runScript(readScript(for: player)).flatMap { parse($0, player: player) }
        })
    }

    /// Sends one button press to a player (blocking: off the main thread). False when the script failed or timed out.
    @discardableResult
    static func send(_ action: MediaAction, to player: MediaPlayer) -> Bool {
        runScript(controlScript(action, for: player)) != nil
    }

    /// Sets the shown player's own volume (blocking: off the main thread). False when the script failed or timed out.
    @discardableResult
    static func setVolume(_ percent: Int, to player: MediaPlayer) -> Bool {
        runScript(volumeScript(percent: percent, for: player)) != nil
    }

    /// Runs AppleScript source with `osascript`; the trimmed output, or nil on any failure (an error, a refused
    /// permission, a player that does not answer within `timeout`, which is then killed).
    static func runScript(_ source: String) -> String? {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        process.arguments = ["-e", source]
        let out = Pipe()
        process.standardOutput = out
        process.standardError = FileHandle.nullDevice
        process.standardInput = FileHandle.nullDevice
        let done = DispatchSemaphore(value: 0)
        process.terminationHandler = { _ in done.signal() }
        do { try process.run() } catch { return nil }
        if done.wait(timeout: .now() + timeout) == .timedOut {
            process.terminate()
            return nil
        }
        guard process.terminationStatus == 0 else { return nil }
        let data = out.fileHandleForReading.readDataToEndOfFile()
        return String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

/// Keeps the latest `NowPlaying` while the island is open. Refreshes immediately, listens for player changes and polls every second; `stop()` cancels the timer and drops any
/// answer still on its way, so a hidden island costs nothing.
@MainActor
final class MediaWatcher {
    static let interval: TimeInterval = 1

    /// Called on the main actor, only with a changed value (nil: nothing to show).
    var onChange: ((NowPlaying?) -> Void)?

    private var timer: Timer?
    private var generation = 0
    private var inFlight = false
    private var last: NowPlaying?
    private var changeObservers: [NSObjectProtocol] = []
    private var refreshPending = false
    private var artTask: Task<Void, Never>?
    private var artKey: String?
    private var artCache: [String: Data] = [:]
    private var artAttempt = Date.distantPast
    /// The level each player had before the speaker button muted it (by bundle id), to restore on unmute.
    private var rememberedLevel: [String: Int] = [:]
    /// A volume change on its way, and the newest one waiting behind it (a slider drag sends the last value, not each).
    private var volumeInFlight = false
    private var pendingVolume: (percent: Int, player: MediaPlayer)?

    var isWatching: Bool { timer != nil }

    func start() {
        guard timer == nil else { return }
        let t = Timer(timeInterval: Self.interval, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.poll() }
        }
        RunLoop.main.add(t, forMode: .common)
        timer = t
        for name in ["com.spotify.client.PlaybackStateChanged", "com.apple.Music.playerInfo"] {
            changeObservers.append(DistributedNotificationCenter.default().addObserver(
                forName: Notification.Name(name), object: nil, queue: .main) { [weak self] _ in
                    Task { @MainActor in self?.poll() }
                })
        }
        poll()
    }

    func stop() {
        timer?.invalidate()
        timer = nil
        generation += 1
        inFlight = false
        refreshPending = false
        for observer in changeObservers { DistributedNotificationCenter.default().removeObserver(observer) }
        changeObservers.removeAll()
        artTask?.cancel()
        artTask = nil
        artKey = nil
        // Keep the last reading through the closing animation; start() refreshes it immediately.
    }

    /// One press of a button, on the player that is shown; the strip is refreshed a moment later.
    func send(_ action: MediaAction) {
        guard timer != nil, let shown = last, let player = MediaPlayer(bundleID: shown.bundleID) else { return }
        DispatchQueue.global(qos: .userInitiated).async {
            MediaControl.send(action, to: player)
            DispatchQueue.main.async {
                Task { @MainActor [weak self] in self?.poll() }
            }
        }
    }

    /// The slider: the shown player's own volume, 0...100. Only that player; never the Mac's output volume.
    func setVolume(_ percent: Int) {
        guard timer != nil, let shown = last, let player = MediaPlayer(bundleID: shown.bundleID) else { return }
        let value = MediaControl.clampPercent(percent)
        if value > 0 { rememberedLevel[shown.bundleID] = nil }      // moved up by hand: nothing stale to restore
        pendingVolume = (value, player)
        flushVolume()
    }

    /// The speaker button: mute (remember the level, go to 0) or unmute (remembered level, or 30 %).
    func toggleMute() {
        guard timer != nil, let shown = last, let player = MediaPlayer(bundleID: shown.bundleID),
              let current = shown.volumePercent else { return }
        let click = MediaControl.speakerClick(currentPercent: current, remembered: rememberedLevel[shown.bundleID])
        rememberedLevel[shown.bundleID] = click.remember
        pendingVolume = (click.newPercent, player)
        var optimistic = shown
        optimistic.volumePercent = click.newPercent      // the icon flips at once; the next read corrects it
        publish(optimistic)
        flushVolume()
    }

    private func flushVolume() {
        guard !volumeInFlight, let next = pendingVolume else { return }
        pendingVolume = nil
        volumeInFlight = true
        DispatchQueue.global(qos: .userInitiated).async {
            MediaControl.setVolume(next.percent, to: next.player)
            Task { @MainActor [weak self] in
                guard let self else { return }
                self.volumeInFlight = false
                if self.pendingVolume != nil {
                    self.flushVolume()
                } else {
                    try? await Task.sleep(for: .milliseconds(400))
                    self.poll()
                }
            }
        }
    }

    private func poll() {
        guard timer != nil else { return }
        guard !inFlight else { refreshPending = true; return }
        let players = MediaControl.runningPlayers()
        guard !players.isEmpty else { publish(nil); loadArtwork(nil); return }
        inFlight = true
        let mine = generation
        DispatchQueue.global(qos: .utility).async {
            let reading = MediaControl.read(players)
            Task { @MainActor [weak self] in
                guard let self, mine == self.generation else { return }
                self.inFlight = false
                var current = reading
                if let key = reading?.artworkKey { current?.artwork = self.artCache[key] }
                self.publish(current)
                self.loadArtwork(current)
                if self.refreshPending { self.refreshPending = false; self.poll() }
            }
        }
    }

    private func loadArtwork(_ track: NowPlaying?) {
        guard let track else { artTask?.cancel(); artTask = nil; artKey = nil; return }
        let key = track.artworkKey
        guard artCache[key] == nil else { return }
        guard artKey != key || (artTask == nil && Date().timeIntervalSince(artAttempt) > 30) else { return }
        artTask?.cancel()
        artKey = key
        artAttempt = Date()
        let mine = generation
        artTask = Task { [weak self] in
            let data = await MediaControl.artworkData(for: track)
            guard !Task.isCancelled, let self, self.generation == mine, self.last?.artworkKey == key else { return }
            self.artTask = nil
            guard let data else { return }
            if self.artCache.count >= 12 { self.artCache.removeAll() }
            self.artCache[key] = data
            var current = self.last
            current?.artwork = data
            self.publish(current)
        }
    }

    private func publish(_ value: NowPlaying?) {
        guard value != last else { return }
        last = value
        onChange?(value)
    }
}
