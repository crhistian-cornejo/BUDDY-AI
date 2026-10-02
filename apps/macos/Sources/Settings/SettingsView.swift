import AppKit
import ServiceManagement
import SwiftUI

/// The core the Settings window talks to (set once at launch).
@MainActor
enum AppServices {
    static var core: BuddyCore?
}

/// Buddy's Settings in a regular titled window (an app without Dock icon cannot rely on the Settings scene's menu).
@MainActor
enum SettingsWindow {
    private static var window: NSWindow?

    static func show() {
        if window == nil {
            let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 680, height: 460),
                             styleMask: [.titled, .closable, .miniaturizable], backing: .buffered, defer: false)
            w.title = "Ajustes de Buddy"
            w.contentView = NSHostingView(rootView: SettingsView())
            w.isReleasedWhenClosed = false
            w.center()
            window = w
        }
        NSApp.activate()
        window?.makeKeyAndOrderFront(nil)
    }
}

/// Buddy's Settings: the system's own Settings window with tabs, plain native controls.
struct SettingsView: View {
    var body: some View {
        if let core = AppServices.core {
            TabView {
                GeneralSettings(core: core)
                    .tabItem { Label("General", systemImage: "gearshape") }
                FolderSettings(core: core)
                    .tabItem { Label("Carpetas", systemImage: "folder") }
                ConnectionSettings(core: core)
                    .tabItem { Label("Conexiones", systemImage: "point.3.connected.trianglepath.dotted") }
                UsageSettings(core: core)
                    .tabItem { Label("Uso", systemImage: "chart.bar") }
                AgentSettings(core: core)
                    .tabItem { Label("Agentes", systemImage: "person.2") }
                BriefingSettings(core: core)
                    .tabItem { Label("Mensajitos", systemImage: "newspaper") }
            }
            .frame(width: 680, height: 460)
        } else {
            Text("Buddy aún no ha arrancado.").padding(40)
        }
    }
}

/// A boolean setting of the core ("false" = off; anything else, including missing, = on).
private struct CoreToggle: View {
    let core: BuddyCore
    let key: String
    let title: String
    let detail: String
    @State private var on = true

    var body: some View {
        Toggle(isOn: Binding(get: { on }, set: { value in
            on = value
            try? core.setSetting(key: key, value: value ? "true" : "false")
        })) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                Text(detail).font(.caption).foregroundStyle(.secondary)
            }
        }
        .onAppear { on = ((try? core.setting(key: key)) ?? nil) != "false" }
    }
}

private struct GeneralSettings: View {
    let core: BuddyCore
    @State private var atLogin = SMAppService.mainApp.status == .enabled
    @State private var loginError: String?

    var body: some View {
        Form {
            Section("Buddy") {
                CoreToggle(core: core, key: "pet.wander", title: "Pasear por la pantalla",
                           detail: "Solo cuando no usas el teclado ni el ratón, y nunca con el chat abierto.")
                Toggle(isOn: Binding(get: { atLogin }, set: setLogin)) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Abrir al iniciar sesión")
                        if let loginError { Text(loginError).font(.caption).foregroundStyle(.red) }
                    }
                }
            }
            Section("Agentes") {
                CoreToggle(core: core, key: "router.cheap", title: "Ahorrar tokens",
                           detail: "Los saludos y la charla corta van al modelo más ligero (Haiku).")
                CoreToggle(core: core, key: "commands.enabled", title: "Permitir que ejecuten comandos",
                           detail: "Siempre con tu clic: cada comando sale en el notch con Permitir o Rechazar.")
            }
        }
        .formStyle(.grouped)
    }

    private func setLogin(_ value: Bool) {
        do {
            if value { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() }
            atLogin = value
            loginError = nil
        } catch {
            loginError = "No se pudo cambiar: \(error.localizedDescription)"
        }
    }
}

private struct FolderSettings: View {
    let core: BuddyCore
    @State private var folders: [AuthorizedFolder] = []
    @State private var selection: String?
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Buddy y sus agentes solo pueden leer (y, si lo marcas, editar) dentro de estas carpetas.")
                .font(.callout)
                .foregroundStyle(.secondary)
            List(selection: $selection) {
                ForEach(folders, id: \.path) { folder in
                    HStack {
                        Image(nsImage: NSWorkspace.shared.icon(forFile: folder.path)).resizable().frame(width: 18, height: 18)
                        Text((folder.path as NSString).abbreviatingWithTildeInPath).lineLimit(1).truncationMode(.middle)
                        Spacer()
                        Toggle("Puede editar", isOn: Binding(get: { folder.canEdit }, set: { edit in
                            folders = (try? core.addFolder(path: folder.path, canEdit: edit)) ?? folders
                        }))
                        .toggleStyle(.checkbox)
                    }
                    .tag(folder.path)
                }
            }
            .overlay {
                if folders.isEmpty {
                    ContentUnavailableView("Sin carpetas", systemImage: "folder.badge.plus",
                                           description: Text("Añade las carpetas que Buddy puede usar."))
                }
            }
            HStack {
                Button { add() } label: { Image(systemName: "plus") }
                    .help("Añadir una carpeta")
                Button { remove() } label: { Image(systemName: "minus") }
                    .disabled(selection == nil)
                    .help("Quitar la carpeta elegida")
                if let error { Text(error).font(.caption).foregroundStyle(.red) }
                Spacer()
            }
        }
        .padding(20)
        .onAppear { folders = (try? core.folders()) ?? [] }
    }

    private func add() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.prompt = "Autorizar"
        panel.message = "Elige una carpeta que Buddy pueda leer"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            folders = try core.addFolder(path: url.path, canEdit: false)
            error = nil
        } catch {
            self.error = String(describing: error)
        }
    }

    private func remove() {
        guard let selection else { return }
        folders = (try? core.removeFolder(path: selection)) ?? folders
        self.selection = nil
    }
}

private struct ConnectionSettings: View {
    let core: BuddyCore
    @State private var status: [HookStatusInfo] = []
    @State private var message: String?

    var body: some View {
        Form {
            Section("Avisos de tus sesiones") {
                ForEach(status, id: \.agent) { item in
                    HStack {
                        ProviderMark(provider: item.agent == "codex" ? "codex" : "claude", size: 14)
                            .padding(6)
                            .background(.black, in: RoundedRectangle(cornerRadius: 6))
                        VStack(alignment: .leading, spacing: 2) {
                            Text(item.name)
                            Text(!item.available ? "No instalado" : item.installed ? "Conectado: sus avisos llegan al notch" : "Sin conectar")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        if item.available {
                            Button(item.installed ? "Desconectar" : "Conectar…") { toggle(item) }
                        }
                    }
                }
            }
            if let message {
                Text(message).font(.caption).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .onAppear { status = core.hooksStatus() }
    }

    /// Shows the exact change before writing it; a dated backup is kept.
    private func toggle(_ item: HookStatusInfo) {
        let install = !item.installed
        guard let preview = try? core.hooksPreview(agent: item.agent, install: install) else { return }
        let alert = NSAlert()
        alert.messageText = install ? "¿Conectar \(item.name) con Buddy?" : "¿Desconectar \(item.name)?"
        alert.informativeText = (install ? "Buddy añadirá sus avisos a " : "Buddy quitará sus avisos de ")
            + "\((preview.path as NSString).abbreviatingWithTildeInPath). Antes guarda una copia; no toca nada más."
        let scroll = NSTextView.scrollableTextView()
        scroll.frame = NSRect(x: 0, y: 0, width: 460, height: 200)
        if let text = scroll.documentView as? NSTextView {
            text.isEditable = false
            text.font = .monospacedSystemFont(ofSize: 10.5, weight: .regular)
            text.string = preview.diff
        }
        alert.accessoryView = scroll
        alert.addButton(withTitle: install ? "Conectar" : "Desconectar")
        alert.addButton(withTitle: "Cancelar")
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        do {
            let backup = try core.hooksWrite(agent: item.agent, install: install, fingerprint: preview.fingerprint)
            message = backup.isEmpty ? "Hecho." : "Hecho. Copia en \((backup as NSString).abbreviatingWithTildeInPath)"
        } catch {
            message = "No se pudo: \(error)"
        }
        status = core.hooksStatus()
    }
}

private struct UsageSettings: View {
    let core: BuddyCore
    @State private var plans: [ProviderUsage] = []
    @State private var report: [TokenReport] = []

    var body: some View {
        Form {
            Section("Tus planes") {
                if plans.isEmpty {
                    Text("Aparecen después del primer chat con Claude o Codex.").foregroundStyle(.secondary)
                }
                ForEach(plans, id: \.provider) { plan in
                    ForEach(plan.windows, id: \.label) { window in
                        LabeledContent("\(plan.name) · \(window.label)") {
                            HStack {
                                ProgressView(value: window.usedPct, total: 100).frame(width: 140)
                                Text("\(Int(window.usedPct.rounded())) %").monospacedDigit().frame(width: 44, alignment: .trailing)
                            }
                        }
                    }
                }
            }
            Section("Tokens de los últimos 7 días") {
                if report.isEmpty {
                    Text("Todavía no hay turnos medidos.").foregroundStyle(.secondary)
                }
                ForEach(report, id: \.feature) { row in
                    LabeledContent {
                        Text("\(Self.k(row.input + row.cached)) entrada · \(Self.k(row.output)) salida")
                            .monospacedDigit()
                            .help(row.costUsd > 0 ? String(format: "Equivaldría a %.2f US$ en la API (tu plan no paga extra)", row.costUsd) : "")
                    } label: {
                        Text(row.feature)
                        Text("\(row.turns) turnos · \(row.provider == "codex" ? "Codex" : "Claude")")
                    }
                }
            }
        }
        .formStyle(.grouped)
        .onAppear {
            core.refreshUsage()
            plans = core.usage()
            report = (try? core.tokenReport(days: 7)) ?? []
        }
    }

    static func k(_ n: Int64) -> String {
        n >= 1_000_000 ? String(format: "%.1f M", Double(n) / 1_000_000) : n >= 1000 ? String(format: "%.1f k", Double(n) / 1000) : "\(n)"
    }
}

/// The «mensajitos»: on or off, what to look for, and a run now.
private struct BriefingSettings: View {
    let core: BuddyCore
    @State private var topics = ""
    @State private var items: [BriefingItem] = []
    @State private var asked = false

    var body: some View {
        Form {
            Section {
                CoreToggle(core: core, key: "briefing.enabled", title: "Mensajitos del día",
                           detail: "A las 8, 13 y 19 h Buddy busca lo nuevo con el modelo más barato (3 búsquedas como mucho). Si no hay nada nuevo, no dice nada.")
            }
            Section("Qué buscar") {
                TextField("Temas", text: $topics, axis: .vertical)
                    .lineLimit(3...6)
                    .labelsHidden()
                    .onSubmit(save)
                HStack {
                    Text("Escribe temas separados por punto y coma. Vacío vuelve a los de siempre.")
                        .font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    Button("Guardar", action: save)
                }
            }
            Section("Hoy") {
                if items.isEmpty {
                    Text(asked ? "Buscando… aparecerán aquí y en el notch." : "Todavía nada hoy.")
                        .foregroundStyle(.secondary)
                }
                ForEach(Array(items.enumerated()), id: \.offset) { _, item in
                    LabeledContent {
                        if let link = item.url, let url = URL(string: link), url.scheme?.hasPrefix("http") == true {
                            Link(destination: url) { Image(systemName: "arrow.up.right.square") }
                                .help("Abrir la fuente")
                        }
                    } label: {
                        Text(item.text)
                        Text(item.topic)
                    }
                }
                HStack {
                    Spacer()
                    Button("Buscar ahora") {
                        save()
                        asked = true
                        core.briefingNow()
                        // With nothing new the core stays quiet: stop saying «Buscando…» after a while.
                        Task { try? await Task.sleep(for: .seconds(120)); asked = false }
                    }
                    .help("Una búsqueda ahora, aunque no sea la hora")
                }
            }
        }
        .formStyle(.grouped)
        .onAppear {
            topics = core.briefingTopics()
            items = core.briefing()
        }
        .onReceive(NotificationCenter.default.publisher(for: .buddyBriefingReady)) { _ in
            items = core.briefing()
            asked = false
        }
    }

    private func save() {
        try? core.setBriefingTopics(topics: topics)
        topics = core.briefingTopics()
    }
}

extension Notification.Name {
    /// Posted when the core announces new «mensajitos» (Settings refreshes its list).
    static let buddyBriefingReady = Notification.Name("buddy.briefingReady")
}

private struct AgentSettings: View {
    let core: BuddyCore
    @State private var agents: [Agent] = []

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            List(agents, id: \.id) { agent in
                HStack(alignment: .top, spacing: 10) {
                    if agent.id == "buddy" { AvatarView(size: 22) } else {
                        Image(systemName: "person.crop.circle").font(.title2).foregroundStyle(.secondary)
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        HStack(spacing: 6) {
                            Text(agent.name).font(.headline)
                            ProviderMark(provider: agent.provider == .codex ? "codex" : "claude", size: 11)
                        }
                        Text(agent.specialty).font(.caption).foregroundStyle(.secondary)
                    }
                }
                .padding(.vertical, 2)
            }
            HStack {
                Text("Cada agente es un archivo agent.md que puedes editar.").font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button("Abrir carpeta de agentes") {
                    let dir = URL(fileURLWithPath: core.dataDir()).appendingPathComponent("agents")
                    NSWorkspace.shared.open(dir)
                }
            }
        }
        .padding(20)
        .onAppear { agents = core.agents() }
    }
}
