import SwiftUI
import ImageIO

// The "now playing" strip under the overview's two cards: album artwork, title, "artist · app" (both on one line,
// clipped with an ellipsis) and previous / play-pause / next. Twin of apps/windows/src/views/media-strip.ts. Everything
// the player reports is shown as text (Text(verbatim:)); one click is one command, nothing repeats.

enum MediaStripMetrics {
    static let height: CGFloat = 34
    /// Between the cards' box and the strip. The overview's 98 pt box already leaves ~12 pt of air under the cards, so
    /// −2 puts the strip 10 pt under them: the same space as between the two cards.
    static let gap: CGFloat = -2
    /// Extra island height while the strip is shown (strip + gap): `islandSize(mediaExtra:)`.
    /// Air under the strip, above the island's bottom edge (the cards alone have 10 pt; the strip needs a little more).
    static let bottom: CGFloat = 8
    static var extra: CGFloat { height + gap + bottom }
}

/// The overview with the strip under its cards while something plays or is paused (AppState.mediaStripShown).
struct OverviewWithMedia: View {
    @ObservedObject var state: AppState

    var body: some View {
        VStack(spacing: MediaStripMetrics.gap) {
            OverviewView(state: state)
                .frame(height: 98)
            if state.mediaStripShown, let playing = state.nowPlaying {
                MediaStripView(playing: playing, onAction: { state.sendMedia($0) },
                               onVolume: { state.setMediaVolume($0) }, onMute: { state.toggleMediaMute() })
                    .frame(height: MediaStripMetrics.height)
                    .transition(.opacity)
            }
        }
    }
}

struct MediaStripView: View {
    let playing: NowPlaying
    let onAction: (MediaAction) -> Void
    /// The player's own volume, 0...100 (never the Mac's). Both stay disabled while the player gave no volume.
    var onVolume: (Int) -> Void = { _ in }
    /// The speaker button: mute (remembering the level) or unmute.
    var onMute: () -> Void = {}
    /// The slider's value while it is being dragged, so a poll arriving mid-drag does not move it.
    @State private var dragging: Double?
    @State private var cover: NSImage?

    private var volumeLevel: Double { dragging ?? Double(playing.volumePercent ?? 0) }

    private var volumeKnown: Bool { playing.volumePercent != nil }

    var body: some View {
        HStack(spacing: 7) {
            ZStack {
                RoundedRectangle(cornerRadius: 6).fill(Color.white.opacity(0.08))
                if let cover {
                    Image(nsImage: cover).resizable().scaledToFill()
                } else {
                    Image(systemName: "music.note").font(.system(size: 14)).foregroundStyle(.secondary)
                }
            }
            .frame(width: 30, height: 30)
            .clipShape(RoundedRectangle(cornerRadius: 6))
            .accessibilityLabel(playing.album.isEmpty ? "Portada del álbum" : "Portada: \(playing.album)")
            VStack(alignment: .leading, spacing: 1) {
                Text(verbatim: playing.headline)
                    .font(.system(size: 11, weight: .semibold))
                    .lineLimit(1)
                    .truncationMode(.tail)
                Text(verbatim: playing.subLine)
                    .font(.system(size: 9.5))
                    .foregroundColor(Color(hex: "#8E939C"))
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .overlay(alignment: .bottom) {
                if let position = playing.positionMs, let duration = playing.durationMs, duration > 0 {
                    GeometryReader { geo in
                        Capsule().fill(Color.white.opacity(0.12))
                            .overlay(alignment: .leading) {
                                Capsule().fill(Color.white.opacity(0.5))
                                    .frame(width: geo.size.width * min(1, max(0, Double(position) / Double(duration))))
                            }
                    }
                    .frame(height: 2).offset(y: 3)
                    .allowsHitTesting(false)
                }
            }
            HStack(spacing: 1) {
                button("Anterior", "backward.fill", enabled: playing.canPrevious, size: 20, icon: 11, action: .previous)
                button(playing.status == .playing ? "Pausa" : "Reproducir",
                       playing.status == .playing ? "pause.fill" : "play.fill",
                       enabled: playing.canPlayPause, size: 28, icon: 12, action: .playPause, filled: true)
                button("Siguiente", "forward.fill", enabled: playing.canNext, size: 20, icon: 11, action: .next)
            }
            volumeControls
        }
        .padding(.leading, 4)
        .padding(.trailing, 6)
        .frame(maxWidth: .infinity)
        .background(RoundedRectangle(cornerRadius: 12).fill(Color.white.opacity(0.06)))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(Color.white.opacity(0.08), lineWidth: 1))
        .foregroundColor(Color(hex: "#F5F6F8"))
        .onChange(of: playing.artwork, initial: true) { _, data in
            guard let data, let source = CGImageSourceCreateWithData(data as CFData, nil),
                  let thumbnail = CGImageSourceCreateThumbnailAtIndex(source, 0, [
                    kCGImageSourceCreateThumbnailFromImageAlways: true,
                    kCGImageSourceThumbnailMaxPixelSize: 120,
                    kCGImageSourceCreateThumbnailWithTransform: true
                  ] as CFDictionary) else { cover = nil; return }
            cover = NSImage(cgImage: thumbnail, size: .zero)
        }
    }

    /// The player's speaker (mute) and a small volume slider. Twin of the Windows strip's, but driven by the player's own
    /// `sound volume`; disabled ("Sin control de volumen") when the player did not report one.
    private var volumeControls: some View {
        let silent = playing.isSilent
        let title = !volumeKnown ? "Sin control de volumen" : silent ? "Activar sonido" : "Silenciar"
        return HStack(spacing: 4) {
            Button(action: onMute) {
                Image(systemName: silent ? "speaker.slash.fill" : "speaker.wave.2.fill")
                    .font(.system(size: 11, weight: .semibold))
                    .frame(width: 20, height: 20)
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .tip(title)
            .accessibilityLabel(title)
            ZStack(alignment: .leading) {
                Capsule().fill(Color.white.opacity(0.2)).frame(height: 3)
                Capsule().fill(Color.white.opacity(0.8))
                    .frame(width: 56 * volumeLevel / 100, height: 3)
                Circle().fill(Color.white).frame(width: 10, height: 10)
                    .offset(x: 46 * volumeLevel / 100)
            }
            .frame(width: 56, height: 24)
            .contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 0)
                .onChanged { value in
                    let percent = min(100, max(0, value.location.x / 56 * 100))
                    dragging = percent
                    onVolume(Int(percent.rounded()))
                }
                .onEnded { _ in dragging = nil })
            .focusable(volumeKnown)
            .onMoveCommand { direction in
                if direction == .left || direction == .down { onVolume(max(0, Int(volumeLevel) - 5)) }
                if direction == .right || direction == .up { onVolume(min(100, Int(volumeLevel) + 5)) }
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Volumen")
            .accessibilityValue("\(Int(volumeLevel)) por ciento")
            .accessibilityAdjustableAction { direction in
                onVolume(min(100, max(0, Int(volumeLevel) + (direction == .increment ? 5 : -5))))
            }
            .tip(volumeKnown ? "Volumen \(Int(volumeLevel))%" : title)
        }
        .disabled(!volumeKnown)
        .opacity(volumeKnown ? 1 : 0.3)
    }

    private func button(_ title: String, _ symbol: String, enabled: Bool, size: CGFloat, icon: CGFloat,
                        action: MediaAction, filled: Bool = false) -> some View {
        Button { onAction(action) } label: {
            Image(systemName: symbol)
                .font(.system(size: icon, weight: .semibold))
                .frame(width: size, height: size)
                .background(Circle().fill(Color.white.opacity(filled ? 0.12 : 0)))
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .opacity(enabled ? 1 : 0.3)
        .tip(title)
        .accessibilityLabel(title)
    }
}

/// Direct transport controls in the global header, outside the conversation.
struct HeaderMusicControls: View {
    @ObservedObject var state: AppState

    var body: some View {
        if state.mediaControl {
            let track = state.nowPlaying
            let available = track?.isVisible == true
            HStack(spacing: 2) {
                transport("Anterior", "backward.end.fill", .previous, available && track?.canPrevious == true)
                transport(track?.status == .playing ? "Pausar" : "Reproducir",
                          track?.status == .playing ? "pause.fill" : "play.fill",
                          .playPause, available && track?.canPlayPause == true)
                transport("Siguiente", "forward.end.fill", .next, available && track?.canNext == true)
            }
            .padding(.horizontal, 4)
            .background(Capsule().fill(Color.white.opacity(0.08)))
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Música")
        }
    }

    private func transport(_ title: String, _ icon: String, _ action: MediaAction, _ enabled: Bool) -> some View {
        Button { state.sendMedia(action) } label: {
            Image(systemName: icon)
                .font(.system(size: 11, weight: .semibold))
                .foregroundColor(Color(hex: "#F5F6F8"))
                .frame(width: 24, height: 26)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .opacity(enabled ? 1 : 0.35)
        .tip(enabled ? title : "Sin reproducción activa", iconOnly: true)
        .accessibilityLabel(title)
    }
}

/// Replaces the mini agents only in the collapsed notch during active playback.
struct CompactAlbumView: View {
    let playing: NowPlaying
    @State private var cover: NSImage?

    var body: some View {
        VStack(spacing: 4) {
            ZStack {
                RoundedRectangle(cornerRadius: 5).fill(Color.white.opacity(0.08))
                if let cover {
                    Image(nsImage: cover).resizable().scaledToFill()
                } else {
                    Image(systemName: "music.note").font(.system(size: 13)).foregroundColor(.secondary)
                }
            }
            .frame(width: 28, height: 28)
            .clipShape(RoundedRectangle(cornerRadius: 5))
        }
        .frame(width: 28, height: 28)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Reproduciendo: \(playing.headline), \(playing.artist)")
        .tip("\(playing.headline) · \(playing.artist)")
        .onChange(of: playing.artwork, initial: true) { _, data in
            guard let data, let source = CGImageSourceCreateWithData(data as CFData, nil),
                  let thumbnail = CGImageSourceCreateThumbnailAtIndex(source, 0, [
                    kCGImageSourceCreateThumbnailFromImageAlways: true,
                    kCGImageSourceThumbnailMaxPixelSize: 200,
                    kCGImageSourceCreateThumbnailWithTransform: true
                  ] as CFDictionary) else { cover = nil; return }
            cover = NSImage(cgImage: thumbnail, size: .zero)
        }
    }
}
