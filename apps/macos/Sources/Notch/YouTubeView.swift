import AppKit
import SwiftUI
import WebKit

/// Same media-card layout as the music player, with controls scoped to the detected browser tab.
struct YouTubeMediaCard: View {
    let video: YouTubeVideo
    let onOpen: () -> Void
    let onToggle: () -> Void
    var body: some View {
        HStack(spacing: 14) {
            AsyncImage(url: video.service == "youtube" ? URL(string: "https://i.ytimg.com/vi/\(video.videoId)/hqdefault.jpg") : nil) { image in
                image.resizable().aspectRatio(contentMode: .fill)
            } placeholder: {
                ZStack { Color.white.opacity(0.1); Image(systemName: "play.rectangle.fill").foregroundStyle(.secondary) }
            }
            .frame(width: 64, height: 64).clipShape(RoundedRectangle(cornerRadius: 12))
            VStack(alignment: .leading, spacing: 6) {
                Text(video.title).font(.system(size: 14, weight: .semibold)).lineLimit(1).tip(video.title)
                Text("\(video.service == "youtube" ? "YouTube" : "Video") · \(video.browser)").font(.system(size: 12)).foregroundStyle(.secondary)
                HStack(spacing: 8) {
                    Text(Self.time(video.seconds)).monospacedDigit()
                    if let duration = video.duration {
                        GeometryReader { geometry in
                            ZStack(alignment: .leading) {
                                Capsule().fill(.white.opacity(0.15))
                                Capsule().fill(.white).frame(width: geometry.size.width * min(video.seconds / duration, 1))
                            }
                        }.frame(height: 4)
                        Text("-" + Self.time(max(0, duration - video.seconds))).monospacedDigit()
                    } else { Text(video.playing ? "Reproduciendo" : "En pausa") }
                }.font(.system(size: 10, weight: .medium)).foregroundStyle(.secondary)
            }.frame(maxWidth: .infinity, alignment: .leading)
            HStack(spacing: 4) {
                Button(action: onToggle) {
                    Image(systemName: video.playing ? "pause.fill" : "play.fill").font(.system(size: 20)).frame(width: 32, height: 32)
                }.buttonStyle(.plain).accessibilityLabel(video.playing ? "Pausar YouTube" : "Reproducir YouTube")
                    .tip(video.playing ? "Pausar en el navegador" : "Reproducir en el navegador")
                if video.service == "youtube" {
                Button {
                    try? AppServices.core?.youtubeOpenFloating(sourceId: video.sourceId, videoId: video.videoId)
                } label: { Image(systemName: "macwindow").font(.system(size: 20)).frame(width: 32, height: 32) }
                    .buttonStyle(.plain).tip("Ver junto a Buddy").accessibilityLabel("Ver junto a Buddy")
                Button(action: onOpen) {
                    Image(systemName: "pip.enter").font(.system(size: 20)).frame(width: 32, height: 32)
                }.buttonStyle(.plain).accessibilityLabel("Reproducir en el notch").tip("Reproducir en el notch")
                } else {
                    Button { try? AppServices.core?.videoBrowserPip(sourceId: video.sourceId, videoId: video.videoId) } label: {
                        Image(systemName: "pip.enter").font(.system(size: 20)).frame(width: 32, height: 32)
                    }.buttonStyle(.plain).tip("Abrir extensión para ver flotante").accessibilityLabel("Ver flotante en Chrome")
                }
                Button { NotificationCenter.default.post(name: .buddyVideoQuestion, object: nil) } label: {
                    Image(systemName: "sparkles").frame(width: 28, height: 32)
                }.buttonStyle(.plain).tip("Preguntar a Gemini sobre este video")
            }
        }.padding(14).frame(height: NotchLayout.playerHeight)
            .background(.white.opacity(0.03), in: RoundedRectangle(cornerRadius: 16))
            .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder(.white.opacity(0.08)))
    }
    private static func time(_ seconds: Double) -> String {
        let value = max(0, Int(seconds)); return String(format: "%d:%02d", value / 60, value % 60)
    }
}

/// The official player stays fully visible; Buddy's actions and status sit outside its viewport.
struct YouTubeNotchView: View {
    let core: BuddyCore
    let video: YouTubeVideo
    @State private var error = ""
    var body: some View {
        VStack(spacing: 8) {
            YouTubeWebPlayer(video: video) { type, code in
                if type == "position" { core.youtubePosition(sourceId: video.sourceId, videoId: video.videoId, seconds: Double(code)) }
                if type == "playing" { core.youtubeStarted(sourceId: video.sourceId, videoId: video.videoId) }
                if type == "error" { error = [101, 150].contains(code) ? "Este video solo se puede ver en YouTube." : "No se pudo reproducir este video. Puedes volver a YouTube." }
                if type == "blocked" { error = "Pulsa reproducir en el video para continuar." }
            }
            .frame(height: 288)
            HStack {
                Text(error.isEmpty ? video.title : error).font(.caption).lineLimit(1).foregroundStyle(.secondary)
                Spacer()
                Button("Junto a Buddy") { try? core.youtubeMove(destination: "floating") }
                Button("Gemini") { NotificationCenter.default.post(name: .buddyVideoQuestion, object: nil) }
                Button("Abrir YouTube") { NSWorkspace.shared.open(URL(string: "https://www.youtube.com/watch?v=\(video.videoId)")!) }
                Button("Cerrar video") { core.youtubeClose() }
            }.buttonStyle(.borderless)
        }.frame(height: 320)
    }
}

struct YouTubeWebPlayer: NSViewRepresentable {
    let video: YouTubeVideo
    let onMessage: (String, Int) -> Void
    func makeCoordinator() -> Coordinator { Coordinator(onMessage: onMessage) }
    func makeNSView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.mediaTypesRequiringUserActionForPlayback = []
        config.userContentController.add(context.coordinator, name: "buddyYouTube")
        let web = WKWebView(frame: .zero, configuration: config)
        web.navigationDelegate = context.coordinator
        context.coordinator.videoID = video.videoId
        let origin = "https://" + (Bundle.main.bundleIdentifier ?? "io.github.crhistian-cornejo.buddy")
        let options = "{width:'100%',height:'100%',videoId:'\(video.videoId)',playerVars:{autoplay:1,playsinline:1,controls:1,start:\(Int(video.seconds)),origin:'\(origin)'},events:{onStateChange:e=>{clearInterval(tick);report('position',Math.floor(e.target.getCurrentTime()));if(e.data===1){report('playing',0);tick=setInterval(()=>report('position',Math.floor(e.target.getCurrentTime())),1000)}},onError:e=>report('error',e.data),onAutoplayBlocked:()=>report('blocked',0)}}"
        let html = """
        <!doctype html><html><head><meta name="referrer" content="strict-origin-when-cross-origin"><style>html,body,#player{margin:0;width:100%;height:100%;background:black;overflow:hidden}</style></head><body><div id="player"></div><script>let tick;const report=(type,code)=>window.webkit.messageHandlers.buddyYouTube.postMessage({type,code});function onYouTubeIframeAPIReady(){new YT.Player('player',\(options));}</script><script src="https://www.youtube.com/iframe_api"></script></body></html>
        """
        web.loadHTMLString(html, baseURL: URL(string: origin))
        return web
    }
    func updateNSView(_ view: WKWebView, context: Context) {}
    static func dismantleNSView(_ view: WKWebView, coordinator: Coordinator) {
        view.stopLoading()
        view.loadHTMLString("<html></html>", baseURL: nil)
        view.configuration.userContentController.removeScriptMessageHandler(forName: "buddyYouTube")
    }
    @MainActor final class Coordinator: NSObject, WKScriptMessageHandler, WKNavigationDelegate {
        let onMessage: (String, Int) -> Void
        var videoID = ""
        init(onMessage: @escaping (String, Int) -> Void) { self.onMessage = onMessage }
        func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
            guard message.frameInfo.isMainFrame, let value = message.body as? [String: Any], let type = value["type"] as? String else { return }
            onMessage(type, value["code"] as? Int ?? 0)
        }
        func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
            if navigationAction.navigationType == .linkActivated, let url = navigationAction.request.url {
                if url.scheme == "https" { NSWorkspace.shared.open(url) }
                decisionHandler(.cancel)
            } else { decisionHandler(.allow) }
        }
        func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: any Error) { onMessage("error", 0) }
    }
}
