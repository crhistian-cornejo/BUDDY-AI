import AppKit
import SwiftUI

extension Notification.Name { static let buddyYouTubeChanged = Notification.Name("buddyYouTubeChanged") }

struct YouTubeSection: View {
    let core: BuddyCore
    @State private var browser = "chrome"
    @State private var status: YouTubeStatus?
    @State private var folder = ""
    @State private var error = ""
    var body: some View {
        Section {
            HStack {
                VStack(alignment: .leading, spacing: 3) {
                    Text(status?.connected == true ? "Conectado ✓" : status?.enabled == true ? "Esperando la extensión" : "Sin configurar")
                    Text(status?.detected?.title ?? "Detecta videos y ofrece verlos en el notch o junto a Buddy.").font(.caption).foregroundStyle(.secondary).lineLimit(2)
                }
                Spacer()
                Button("Comprobar") { refresh() }
            }
            Picker("Navegador", selection: $browser) { Text("Chrome").tag("chrome"); Text("Edge").tag("edge") }
            HStack {
                Button("Configurar extensión…") { prepare() }
                if status?.enabled == true { Button("Desactivar") { try? core.youtubeEnable(enabled: false); refresh() } }
            }
            if !folder.isEmpty {
                Text("1. En el navegador, abre Extensiones y activa Modo desarrollador.\n2. Pulsa Cargar descomprimida y selecciona la carpeta preparada.\n3. Recarga tu pestaña de YouTube. Buddy comprobará la conexión.")
                    .font(.callout).fixedSize(horizontal: false, vertical: true)
                HStack {
                    Button("Abrir extensiones") { openBrowser() }
                    Button("Mostrar carpeta") { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: folder)]) }
                    Button("Copiar ubicación") { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(folder, forType: .string) }
                }
            }
            if !error.isEmpty { Text(error).foregroundStyle(.red).font(.caption) }
        } header: { Label("YouTube", systemImage: "play.rectangle") }
        footer: { Text("Gratis, sin claves API ni cuentas adicionales. YouTube está habilitado; puedes activar otros sitios desde el icono de la extensión. El navegador requiere que tú la habilites una vez.").font(.caption) }
        .onAppear { refresh(); folder = core.youtubeExtensionPath() }
        .onReceive(NotificationCenter.default.publisher(for: .buddyYouTubeChanged)) { _ in refresh() }
    }
    private func refresh() { status = core.youtubeStatus() }
    private func prepare() {
        do { folder = try core.youtubePrepare(browser: browser); error = ""; refresh(); openBrowser() }
        catch { self.error = String(describing: error) }
    }
    private func openBrowser() {
        let bundle = browser == "chrome" ? "com.google.Chrome" : "com.microsoft.edgemac"
        guard let app = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundle) else { error = "Instala \(browser == "chrome" ? "Chrome" : "Edge") o elige el navegador que ya usas."; return }
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.arguments = [browser == "chrome" ? "chrome://extensions" : "edge://extensions"]
        NSWorkspace.shared.openApplication(at: app, configuration: configuration)
    }
}
