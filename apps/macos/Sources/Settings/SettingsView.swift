import AppKit
import ServiceManagement
import SwiftUI

/// The core the Settings window talks to (set once at launch).
@MainActor
enum AppServices {
    static var core: BuddyCore?
    static var notchSystem: NotchSystemMonitor?
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
    /// BUDDY_DEBUG_SETTINGS=<tab> opens that tab (debug builds), to look at it without clicking.
    @State private var tab = ProcessInfo.processInfo.environment["BUDDY_DEBUG_SETTINGS"].flatMap { ["general", "carpetas", "conexiones", "uso", "agentes"].contains($0) ? $0 : nil } ?? "general"

    var body: some View {
        if let core = AppServices.core {
            TabView(selection: $tab) {
                GeneralSettings(core: core)
                    .tabItem { Label("General", systemImage: "gearshape") }.tag("general")
                FolderSettings(core: core)
                    .tabItem { Label("Carpetas", systemImage: "folder") }.tag("carpetas")
                ConnectionSettings(core: core)
                    .tabItem { Label("Conexiones", systemImage: "point.3.connected.trianglepath.dotted") }.tag("conexiones")
                UsageSettings(core: core)
                    .tabItem { Label("Uso", systemImage: "chart.bar") }.tag("uso")
                AgentSettings(core: core)
                    .tabItem { Label("Agentes", systemImage: "person.2") }.tag("agentes")
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

/// What the team remembers about the user: each note can be deleted.
private struct MemorySettings: View {
    let core: BuddyCore
    @State private var notes: [String] = []

    var body: some View {
        Section {
            if notes.isEmpty {
                Text("Todavía nada. Cuando le cuentes a Buddy o a Niko algo que se repite (un gasto fijo, cómo prefieres las cosas), lo anotan aquí.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            ForEach(notes, id: \.self) { note in
                HStack(alignment: .firstTextBaseline) {
                    Text(note).fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 8)
                    Button {
                        core.forgetMemory(note: note)
                        notes = core.memoryNotes()
                    } label: { Image(systemName: "trash") }
                        .buttonStyle(.borderless)
                        .help("Olvidar esto")
                        .accessibilityLabel("Olvidar: \(note)")
                }
            }
        } header: {
            Text("Memoria")
        } footer: {
            Text("Lo que Buddy y sus agentes recuerdan de ti entre conversaciones. Se guarda solo en este equipo.")
                .font(.caption).foregroundStyle(.secondary)
        }
        .onAppear { notes = core.memoryNotes() }
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
            MemorySettings(core: core)
            ModelSettings(core: core)
            if let system = AppServices.notchSystem { NotchSystemSettings(system: system) }
            Section("Agentes") {
                CoreToggle(core: core, key: "commands.enabled", title: "Permitir que ejecuten comandos",
                           detail: "Siempre con tu clic: cada comando sale en el notch con Permitir o Rechazar.")
            }
            AlwaysRulesSection(core: core)
            BriefingSettings(core: core)
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

/// Which model answers Buddy: one fixed, or the router choosing by what is asked (rules, no tokens spent).
private struct ModelSettings: View {
    let core: BuddyCore
    @State private var config: RouterConfig?
    @State private var sample = ""

    private static let efforts = [("low", "Bajo"), ("medium", "Medio"), ("high", "Alto")]
    private static let details: [Tier: String] = [
        .light: "Saludos y charla corta.",
        .normal: "La mayoría de preguntas.",
        .deep: "Análisis, comparaciones, planes, mensajes largos.",
        .code: "Código, errores y archivos de programación.",
        .work: "Word, Excel, presentaciones, informes y archivos.",
        .math: "Cálculos, ecuaciones, estadística y demostraciones.",
    ]

    var body: some View {
        Section {
            if let config {
                Picker("Modelo", selection: Binding(get: { config.mode }, set: { mode in
                    try? core.setRouterMode(mode: mode)
                    reload()
                })) {
                    Text("Automático (según lo que pidas)").tag("auto")
                    Divider()
                    ForEach(config.models, id: \.id) { Text($0.name).tag($0.id) }
                }
                if config.mode == "auto" {
                    ForEach(config.tiers, id: \.label) { choice in
                        LabeledContent {
                            HStack(spacing: 6) {
                                Picker("Modelo", selection: Binding(get: { choice.model }, set: { save(choice, model: $0) })) {
                                    ForEach(config.models, id: \.id) { Text($0.name).tag($0.id) }
                                }
                                .labelsHidden()
                                .frame(width: 130)
                                Picker("Esfuerzo", selection: Binding(get: { choice.effort }, set: { save(choice, effort: $0) })) {
                                    ForEach(Self.efforts, id: \.0) { Text($0.1).tag($0.0) }
                                }
                                .labelsHidden()
                                .frame(width: 84)
                                .help("Cuánto piensa el modelo antes de responder")
                            }
                        } label: {
                            Text(choice.label)
                            Text(Self.details[choice.tier] ?? "")
                        }
                    }
                }
                TextField("Probar", text: $sample, prompt: Text("Escribe un pedido para ver qué modelo usaría"))
                if !sample.trimmingCharacters(in: .whitespaces).isEmpty {
                    Text(core.routerPreview(text: sample))
                        .font(.callout.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
            }
        } header: {
            Text("Modelo de Buddy")
        } footer: {
            Text("En un mensaje puedes elegir tú: «con opus», «usa gpt», «con sonnet». Los especialistas como PARLEY usan su propio modelo.")
                .font(.caption).foregroundStyle(.secondary)
        }
        .onAppear(perform: reload)
    }

    private func reload() { config = core.routerConfig() }

    private func save(_ choice: TierChoice, model: String? = nil, effort: String? = nil) {
        try? core.setRouterTier(tier: choice.tier, model: model ?? choice.model, effort: effort ?? choice.effort)
        reload()
    }
}

/// The commands allowed for good from a card («Permitir siempre»), with who may run them; each can be removed.
private struct AlwaysRulesSection: View {
    let core: BuddyCore
    @State private var rules: [AlwaysRule] = []

    var body: some View {
        Section {
            if rules.isEmpty {
                Text("Ninguno. En la tarjeta de un comando, «Permitir siempre» lo añade aquí.")
                    .font(.callout).foregroundStyle(.secondary)
            }
            ForEach(rules, id: \.self) { rule in
                LabeledContent {
                    Button {
                        core.removeAlwaysRule(agent: rule.agent, prefix: rule.prefix)
                        rules = core.alwaysRules()
                    } label: {
                        Image(systemName: "minus.circle")
                    }
                    .buttonStyle(.borderless)
                    .help("Volver a preguntar por este comando")
                } label: {
                    Text(rule.prefix + " …").font(.body.monospaced())
                    Text(AgentNames.name(rule.agent))
                }
            }
        } header: {
            Text("Comandos permitidos siempre")
        } footer: {
            Text("Solo comandos simples (sin «|», «;», «&&» ni redirecciones) y nunca rm, sudo, curl o parecidos.")
                .font(.caption).foregroundStyle(.secondary)
        }
        .onAppear { rules = core.alwaysRules() }
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
                        ProviderMark(provider: AgentNames.mark(item.agent), size: 14)
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
            YouTubeSection(core: core)
            SpotifySection(core: core)
            TelegramSection(core: core)
            TelegramAccountSection(core: core)
            ParleyOddsSection(core: core)
            ConnectorsSection(core: core)
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

/// Spotify's search for Buddy: the user's own app (Client ID in settings, Client Secret only in the Keychain).
private struct SpotifySection: View {
    let core: BuddyCore
    @State private var clientID = ""
    @State private var secret = ""
    @State private var connected = ""
    @State private var busy = false
    @State private var error: String?
    @State private var playlists = false
    @State private var asking = false

    var body: some View {
        Section {
            if connected.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Para que Buddy busque y ponga música dentro de Spotify (sin buscar en la web):")
                    Text(verbatim: "1. Abre el panel de desarrolladores y crea una app (cualquier nombre; en «Redirect URI» pon http://127.0.0.1:8888).")
                    Text("2. Marca «Web API», guarda y copia aquí su Client ID y su Client Secret.")
                }
                .font(.caption)
                .foregroundStyle(.secondary)
                Link(destination: URL(string: "https://developer.spotify.com/dashboard")!) {
                    Label("Abrir el panel de Spotify", systemImage: "arrow.up.right.square")
                }
                TextField("Client ID", text: $clientID, prompt: Text("Pégalo aquí"))
                SecureField("Client Secret", text: $secret, prompt: Text("Pégalo aquí"))
                HStack {
                    if let error { Text(error).font(.caption).foregroundStyle(.red) }
                    Spacer()
                    if busy { ProgressView().controlSize(.small) }
                    Button("Conectar", action: connect)
                        .disabled(busy || clientID.trimmingCharacters(in: .whitespaces).isEmpty || secret.isEmpty)
                }
            } else {
                LabeledContent {
                    Button("Desconectar") {
                        try? core.spotifyDisconnect()
                        connected = core.spotifyClientId()
                    }
                } label: {
                    Text("Conectado")
                    Text("App \(connected.prefix(6))… · el secreto está en tu Llavero")
                }
                LabeledContent {
                    if playlists {
                        Button("Quitar permiso") {
                            core.spotifyForgetPlaylists()
                            playlists = core.spotifyCanCreatePlaylists()
                        }
                    } else {
                        if asking { ProgressView().controlSize(.small) }
                        Button("Permitir crear playlists", action: allowPlaylists).disabled(asking)
                    }
                } label: {
                    Text("Playlists")
                    Text(playlists ? "Buddy puede crear playlists privadas en tu cuenta"
                         : asking ? "Acepta en la página de Spotify que se abrió"
                         : "Spotify te pedirá permiso en el navegador (solo crear y llenar playlists)")
                }
                if let error { Text(error).font(.caption).foregroundStyle(.red) }
            }
        } header: {
            Label("Spotify", systemImage: "music.note")
        } footer: {
            Text("Spotify exige Premium en la cuenta dueña de la app. Reproducir no lo necesita: Buddy usa la app de Spotify de tu Mac.")
                .font(.caption).foregroundStyle(.secondary)
        }
        .onAppear {
            connected = core.spotifyClientId()
            playlists = core.spotifyCanCreatePlaylists()
        }
        // The core says when the browser came back (it reuses the «something changed» notice).
        .onReceive(NotificationCenter.default.publisher(for: .buddyUsageChanged).receive(on: DispatchQueue.main)) { _ in
            let now = core.spotifyCanCreatePlaylists()
            if asking && now { asking = false }
            playlists = now
        }
    }

    /// Opens Spotify's own page; the answer comes back to this Mac only (127.0.0.1).
    private func allowPlaylists() {
        error = nil
        do {
            let page = try core.spotifyAuthorize()
            guard let url = URL(string: page) else { return }
            asking = true
            NSWorkspace.shared.open(url)
            // If the page is closed without answering, the button comes back.
            DispatchQueue.main.asyncAfter(deadline: .now() + 300) { asking = false }
        } catch {
            if case let CoreError.Hooks(message) = error { self.error = message } else { self.error = String(describing: error) }
        }
    }

    private func connect() {
        busy = true
        error = nil
        let (id, key) = (clientID, secret)
        Task.detached {
            let result = Result { try core.spotifyConnect(clientId: id, clientSecret: key) }
            await MainActor.run {
                busy = false
                switch result {
                case .success:
                    secret = ""
                    connected = core.spotifyClientId()
                case let .failure(failure):
                    if case let CoreError.Hooks(message) = failure { error = message } else { error = String(describing: failure) }
                }
            }
        }
    }
}

struct UsageSettings: View {
    let core: BuddyCore
    @State private var plans: [ProviderUsage] = []
    @State private var report: [TokenReport] = []
    @State private var activity: [TokenDay] = []

    var body: some View {
        Form {
            Section {
                UsageCalendar(days: activity)
                    .listRowInsets(EdgeInsets())
            }
            Section("Tus planes") {
                if plans.isEmpty {
                    Text("Aparecen después del primer chat con Claude, Codex o Gemini.").foregroundStyle(.secondary)
                }
                if !plans.contains(where: { $0.provider == "antigravity" && !$0.windows.isEmpty }) {
                    LabeledContent("Gemini") {
                        Text("Cuotas no disponibles").foregroundStyle(.secondary)
                    }
                }
                ForEach(plans, id: \.provider) { plan in
                    ForEach(plan.windows, id: \.label) { window in
                        LabeledContent {
                            HStack {
                                ProgressView(value: window.usedPct, total: 100).frame(width: 140)
                                Text("\(Int(window.usedPct.rounded())) %").monospacedDigit().frame(width: 44, alignment: .trailing)
                            }
                        } label: {
                            Text("\(plan.name) · \(window.label)")
                            if let reset = window.resetsAt {
                                Text("Se reinicia \(Date(timeIntervalSince1970: TimeInterval(reset)).formatted(.dateTime.weekday().day().month().hour().minute()))")
                                    .font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
            Section("Tokens de los últimos 7 días") {
                if report.isEmpty {
                    Text("Todavía no hay turnos medidos.").foregroundStyle(.secondary)
                }
                ForEach(report, id: \.self) { row in
                    LabeledContent {
                        Text("\(Self.k(row.input + row.cached)) entrada · \(Self.k(row.output)) salida")
                            .monospacedDigit()
                            .help(row.costUsd > 0 ? String(format: "Equivaldría a %.2f US$ en la API (tu plan no paga extra)", row.costUsd) : "")
                    } label: {
                        Text(row.feature)
                        Text("\(row.turns) turnos · \(row.provider == "codex" ? "Codex" : row.provider == "antigravity" ? "Gemini" : "Claude")")
                    }
                }
            }
        }
        .formStyle(.grouped)
        .onAppear {
            core.refreshUsage()
            reload()
        }
        .onReceive(NotificationCenter.default.publisher(for: .buddyUsageChanged)) { _ in reload() }
    }

    private func reload() {
        plans = core.usage()
        report = (try? core.tokenReport(days: 7)) ?? []
        activity = (try? core.tokenActivity(days: 371)) ?? []
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
        Group {
            Section {
                CoreToggle(core: core, key: "briefing.enabled", title: "Novedades del día",
                           detail: "A las 8, 16 y 19 h Buddy busca lo nuevo con el modelo más barato (3 búsquedas como mucho). Si no hay nada nuevo, no dice nada. Al empezar otro día se borran las noticias anteriores de todos los paneles.")
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
    static let buddyUsageChanged = Notification.Name("buddy.usageChanged")
}

private struct AgentSettings: View {
    let core: BuddyCore
    @State private var agents: [Agent] = []
    @State private var catalog: [PermissionInfo] = []
    @State private var models: [ModelOption] = []
    @State private var selectedAgent = "buddy"

    var body: some View {
        VStack(spacing: 0) {
            // One tab per agent, drawn here: the system's segmented picker collapsed to a sliver in this window.
            HStack(spacing: 6) {
                ForEach(agents, id: \.id) { agent in
                    Button { selectedAgent = agent.id } label: {
                        HStack(spacing: 6) {
                            AgentAvatarView(agentId: agent.id, size: 18)
                            Text(agent.name).lineLimit(1)
                        }
                        .font(.system(size: 13, weight: selectedAgent == agent.id ? .semibold : .regular))
                        .padding(.horizontal, 12)
                        .frame(height: 30)
                        .frame(maxWidth: .infinity)
                        .background(selectedAgent == agent.id ? Color.accentColor.opacity(0.22) : Color.primary.opacity(0.06),
                                    in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(agent.name)
                    .accessibilityAddTraits(selectedAgent == agent.id ? .isSelected : [])
                }
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 12)
            Form {
                ForEach(agents.filter { $0.id == selectedAgent }, id: \.id) { agent in
                    Section {
                        if agent.id == "buddy" {
                            LabeledContent("Modelo") {
                                Text("El de «Modelo de Buddy» en General").foregroundStyle(.secondary)
                            }
                        } else {
                            Picker("Modelo", selection: Binding(get: { modelSelection(agent) }, set: { model in
                                try? core.setAgentModel(agentId: agent.id, model: model)
                                reload()
                            })) {
                                Text("Automático (el router decide)").tag("auto")
                                Divider()
                                ForEach(models, id: \.id) { Text($0.name).tag($0.id) }
                                if let own = agent.model, own != "auto", !models.contains(where: { $0.id == own }) {
                                    Divider()
                                    Text("El de su archivo (\(own))").tag("")
                                }
                            }
                        }
                        LabeledContent("Puede") {
                            Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 6) {
                                ForEach(Array(stride(from: 0, to: catalog.count, by: 2)), id: \.self) { i in
                                    GridRow {
                                        ForEach(catalog[i..<min(i + 2, catalog.count)], id: \.id) { permission in
                                            Toggle(permission.name, isOn: Binding(
                                                get: { agent.permissions.contains(permission.id) },
                                                set: { on in toggle(agent, permission.id, on) }))
                                            .toggleStyle(.checkbox)
                                            .help(permission.detail)
                                        }
                                    }
                                }
                            }
                        }
                        AgentFaceEditor(core: core, agent: agent)
                    } header: {
                        HStack(spacing: 8) {
                            AgentAvatarView(agentId: agent.id, size: 18)
                            Text(agent.name)
                            Text(agent.specialty).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        }
                    }
                }
                if selectedAgent == "niko" {
                    NikoSettings(core: core, embedded: true)
                }
                Section {
                    HStack {
                        Text("Cada agente es un archivo agent.md que puedes editar. Ejecutar comandos siempre pide tu clic.")
                            .font(.caption).foregroundStyle(.secondary)
                        Spacer()
                        Button("Abrir carpeta de agentes") {
                            NSWorkspace.shared.open(URL(fileURLWithPath: core.dataDir()).appendingPathComponent("agents"))
                        }
                    }
                }
            }
            .formStyle(.grouped)
        }
        .onAppear(perform: reload)
    }

    /// "auto", a router model id, or "" (the model written in its agent.md).
    private func modelSelection(_ agent: Agent) -> String {
        guard let model = agent.model else { return "" }
        return model == "auto" || models.contains(where: { $0.id == model }) ? model : ""
    }

    private func toggle(_ agent: Agent, _ permission: String, _ on: Bool) {
        var list = agent.permissions.filter { $0 != permission }
        if on { list.append(permission) }
        try? core.setAgentPermissions(agentId: agent.id, permissions: list)
        reload()
    }

    private func reload() {
        agents = core.agents()
        catalog = core.agentPermissionCatalog()
        models = core.routerConfig().models
    }
}
