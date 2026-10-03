import AppKit
import SwiftUI

extension Notification.Name {
    /// Posted when the core says Niko's state changed (a review started or ended, a setting).
    static let buddyNikoChanged = Notification.Name("buddy.nikoChanged")
}

/// Settings › Niko · finanzas: the personal-finance agent. Gmail and Notion are the user's own claude.ai connectors
/// (Anthropic holds those keys, never Buddy); the review of the mail runs only on the device where it is switched on.
/// The core does all of it; this only shows and asks. Twin of the Windows `settings/niko.ts`.
struct NikoSettings: View {
    let core: BuddyCore
    var embedded = false
    @State private var status: NikoStatus?
    @State private var accounts: [AccountStatus] = []
    @State private var checking = false
    @State private var senders = ""
    @State private var parent = ""
    @State private var error: String?
    @State private var saved: String?
    @State private var mail: MailWatchStatus?
    @State private var mailAddress = ""
    @State private var mailPassword = ""
    @State private var mailError: String?
    @State private var connectingMail = false

    private static let intervals: [UInt32] = [10, 20, 30, 60]
    private static let connectors = URL(string: "https://claude.ai/settings/connectors")!

    var body: some View {
        if embedded { content } else { Form { content }.formStyle(.grouped) }
    }

    private var content: some View {
        Group {
            Section {
                HStack(alignment: .top, spacing: 12) {
                    AgentAvatarView(agentId: "niko", size: 36)
                    VStack(alignment: .leading, spacing: 4) {
                        Text("«Entiende tu plata, no solo la anotes»").font(.headline)
                        Text("Niko anota en tu Notion cada movimiento: los correos de tus bancos y apps, lo que le escribas («gasté 45 en almuerzo») y las fotos de tus vouchers de Yape o Plin. Nunca mueve dinero ni responde correos.")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
                if let status, !status.agentReady {
                    Label("Niko necesita el permiso «Cuentas» (Ajustes › Agentes).", systemImage: "exclamationmark.triangle")
                        .font(.caption).foregroundStyle(.orange)
                }
            }

            Section {
                if accounts.isEmpty {
                    HStack {
                        Text(checking ? "Comprobando Claude y GPT…" : "No se pudieron comprobar las conexiones.")
                            .foregroundStyle(.secondary)
                        Spacer()
                        if checking { ProgressView().controlSize(.small) }
                    }
                }
                ForEach(accounts, id: \.id) { account in
                    LabeledContent {
                        if account.state != "connected" {
                            Link(account.id.hasPrefix("codex:") ? "Revisar en ChatGPT" : "Autorizar en Claude", destination: account.id.hasPrefix("codex:") ? URL(string: "https://chatgpt.com")! : Self.connectors)
                        }
                    } label: {
                        Label {
                            Text(account.name)
                            Text(Self.stateText(account)).foregroundStyle(account.state == "connected" ? Color.secondary : Color.orange)
                        } icon: {
                            Image(systemName: account.state == "connected" ? "checkmark.circle.fill" : "exclamationmark.circle")
                                .foregroundStyle(account.state == "connected" ? .green : .orange)
                        }
                    }
                }
                HStack {
                    Spacer()
                    Button("Volver a comprobar", action: checkAccounts).disabled(checking)
                }
            } header: {
                Text("Claude y GPT · cambio automático")
            } footer: {
                Text("Usa tus conexiones de Claude y ChatGPT. Si uno se queda sin cuota, Niko continúa con el otro y conserva las mismas bases de Notion. Buddy no guarda esas claves. El acceso al correo se confirma al revisarlo.")
                    .font(.caption).foregroundStyle(.secondary)
            }

            Section {
                if let mail, !mail.email.isEmpty {
                    LabeledContent {
                        Button("Desconectar") {
                            core.nikoMailDisconnect()
                            reload()
                        }
                    } label: {
                        Label {
                            Text(mail.email)
                            Text(mail.connected ? "Vigilando la bandeja" : mail.error.isEmpty ? "Conectando…" : mail.error)
                                .foregroundStyle(mail.connected || mail.error.isEmpty ? Color.secondary : Color.orange)
                        } icon: {
                            Image(systemName: mail.connected ? "bolt.circle.fill" : "exclamationmark.circle")
                                .foregroundStyle(mail.connected ? .green : .orange)
                        }
                    }
                } else {
                    // The examples are grey: in the accent colour they read as an address already filled in.
                    TextField("Tu Gmail", text: $mailAddress, prompt: Text("escribe tu dirección").foregroundStyle(.tertiary))
                        .textContentType(.username)
                        .foregroundStyle(.primary)
                    SecureField("Contraseña de aplicación", text: $mailPassword, prompt: Text("16 letras que te da Google").foregroundStyle(.tertiary))
                    if mailAddress.isEmpty && !mailPassword.isEmpty {
                        Text("Falta escribir tu dirección de Gmail arriba.").font(.caption).foregroundStyle(.orange)
                    }
                    if let mailError { Text(mailError).font(.caption).foregroundStyle(.red) }
                    HStack {
                        Link("Crear una contraseña de aplicación", destination: URL(string: "https://myaccount.google.com/apppasswords")!)
                        Spacer()
                        if connectingMail { ProgressView().controlSize(.small) }
                        Button("Conectar", action: connectMail)
                            .disabled(connectingMail || mailAddress.isEmpty || mailPassword.isEmpty)
                    }
                }
            } header: {
                Text("Avisos al momento")
            } footer: {
                Text("Buddy se conecta a tu Gmail solo para leer (IMAP): ve el remitente, el asunto y el inicio del texto de cada correo nuevo, y avisa en segundos cuando es un cobro, un pago o una suscripción. No abre adjuntos ni enlaces, y no puede enviar, borrar ni marcar correos. La contraseña de aplicación se guarda en el Llavero y la puedes revocar cuando quieras en tu cuenta de Google.")
                    .font(.caption).foregroundStyle(.secondary)
            }

            Section {
                Stepper(value: Binding(get: { Int(status?.cycleDay ?? 1) }, set: { day in
                    try? core.nikoSetCycleDay(day: UInt32(day))
                    reload()
                }), in: 1...28) {
                    Text("Tu mes empieza el día \(status?.cycleDay ?? 1)")
                }
            } header: {
                Text("Ciclo del mes")
            } footer: {
                Text("Si tus tarjetas cierran el 25, pon 26: lo que compres desde ese día cuenta para el mes siguiente, y «este mes» va del 26 al 25.")
                    .font(.caption).foregroundStyle(.secondary)
            }

            Section("Revisión del correo") {
                Toggle(isOn: Binding(get: { status?.enabled ?? false }, set: { on in
                    core.nikoSetEnabled(on: on)
                    reload()
                })) {
                    Text("Niko revisa el correo en este equipo")
                    Text("Déjalo encendido en un solo equipo (este Mac o tu PC). Usa Haiku o GPT Luna con esfuerzo bajo para revisar rápido y gastar poco.")
                }
                Picker("Cada", selection: Binding(get: { status?.interval ?? 20 }, set: { minutes in
                    try? core.nikoSetInterval(minutes: minutes)
                    reload()
                })) {
                    ForEach(Self.intervals, id: \.self) { Text("\($0) minutos").tag($0) }
                }
                VStack(alignment: .leading, spacing: 6) {
                    Text("Remitentes (dominios o correos)")
                    TextField("Remitentes", text: $senders, axis: .vertical)
                        .lineLimit(2...5)
                        .labelsHidden()
                    HStack {
                        Text("Separados por comas. Vacío vuelve a los bancos y apps de siempre.")
                            .font(.caption).foregroundStyle(.secondary)
                        Spacer()
                        Button("Guardar") {
                            senders = core.nikoSetSenders(senders: senders)
                            saved = "Remitentes guardados."
                        }
                    }
                }
                Toggle(isOn: Binding(get: { status?.telegram ?? false }, set: { on in
                    core.nikoSetTelegram(on: on)
                    reload()
                })) {
                    Text("Avisarme también por Telegram")
                    Text("Lo que anota y los avisos de presupuesto, en tu chat vinculado.")
                }
            }

            Section("Notion") {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Página donde Niko guarda todo")
                    HStack {
                        TextField("Enlace de la página", text: $parent, prompt: Text(verbatim: "https://www.notion.so/Buddy-Finanzas-…"))
                            .labelsHidden()
                        Button("Guardar", action: saveParent)
                    }
                    Text("Vacío: Niko busca una página llamada «Buddy · Finanzas». Compártela con el conector de Notion.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                HStack {
                    Button("Abrir dashboard") {
                        if let link = status?.dashboardUrl, let url = URL(string: link) { NSWorkspace.shared.open(url) }
                    }
                    .disabled((status?.dashboardUrl ?? "").isEmpty)
                    Button("Actualizar dashboard") { core.nikoRefreshDashboard() }
                        .disabled((status?.dashboardUrl ?? "").isEmpty)
                        .help("Lee el mes en Notion y vuelve a escribir los totales")
                    Spacer()
                }
            }

            Section("Última revisión") {
                HStack {
                    Text(lastRunText).foregroundStyle(status?.lastOk == false && (status?.lastRunAt ?? 0) > 0 ? .orange : .secondary)
                    Spacer()
                    if status?.running == true { ProgressView().controlSize(.small) }
                    Button("Revisar ahora") { core.nikoReviewNow() }
                        .disabled(status?.running == true)
                }
                ForEach(Array((status?.recent ?? []).enumerated()), id: \.offset) { _, record in
                    LabeledContent {
                        Text(Self.date(record.at)).font(.caption).foregroundStyle(.secondary)
                    } label: {
                        Text("\(Self.money(record.monto, record.moneda)) · \(record.comercio.isEmpty ? record.concepto : record.comercio)")
                        Text("\(record.tipo) · \(record.categoria)")
                    }
                }
            }

            if let error {
                Text(error).font(.caption).foregroundStyle(.red)
            } else if let saved {
                Text(saved).font(.caption).foregroundStyle(.secondary)
            }
        }
        .onAppear {
            reload()
            senders = status?.senders ?? ""
            parent = status?.parentPage ?? ""
            checkAccounts()
        }
        .onReceive(NotificationCenter.default.publisher(for: .buddyNikoChanged).receive(on: DispatchQueue.main)) { _ in reload() }
    }

    private var lastRunText: String {
        guard let status, status.lastRunAt > 0 else { return status?.running == true ? "Revisando…" : "Todavía no revisó el correo." }
        if status.running { return "Revisando…" }
        let when = RelativeDateTimeFormatter().localizedString(for: Date(timeIntervalSince1970: TimeInterval(status.lastRunAt)), relativeTo: Date())
        if !status.lastOk { return "\(when.capitalized): \(status.lastError)" }
        return status.lastRecorded == 0 ? "\(when.capitalized): nada nuevo." : "\(when.capitalized): anotó \(status.lastRecorded)."
    }

    private func reload() {
        status = core.nikoStatus()
        mail = core.nikoMailStatus()
    }

    /// Gmail is asked off the main thread (it can take a few seconds); the password leaves the field either way.
    private func connectMail() {
        let (core, address, password) = (core, mailAddress, mailPassword)
        mailPassword = ""
        mailError = nil
        connectingMail = true
        Task.detached {
            let failure: String?
            do {
                try core.nikoMailConnect(email: address, password: password)
                failure = nil
            } catch {
                failure = Self.message(error)
            }
            await MainActor.run {
                connectingMail = false
                mailError = failure
                reload()
            }
        }
    }

    private nonisolated static func message(_ error: Error) -> String {
        if case let CoreError.Hooks(text) = error { return text }
        return error.localizedDescription
    }

    private func checkAccounts() {
        checking = true
        let core = core
        Task.detached {
            let list = core.nikoAccounts()
            await MainActor.run {
                accounts = list
                checking = false
            }
        }
    }

    private func saveParent() {
        do {
            try core.nikoSetParent(page: parent)
            error = nil
            saved = "Página guardada."
        } catch let CoreError.Hooks(message) {
            error = message
        } catch {
            self.error = String(describing: error)
        }
        reload()
    }

    private static func stateText(_ account: AccountStatus) -> String {
        switch account.state {
        case "connected": return "Conectado"
        case "needs-auth": return "Falta autorizar"
        case "missing": return "No disponible en esta conexión"
        default: return "No responde"
        }
    }

    /// «S/. 1,234.50», «US$ 12.99» (the core formats the notices the same way).
    static func money(_ amount: Double, _ currency: String) -> String {
        let formatter = NumberFormatter()
        formatter.locale = Locale(identifier: "es_PE")
        formatter.numberStyle = .decimal
        formatter.minimumFractionDigits = 2
        formatter.maximumFractionDigits = 2
        formatter.decimalSeparator = "."
        formatter.groupingSeparator = ","
        formatter.usesGroupingSeparator = true
        let number = formatter.string(from: NSNumber(value: amount)) ?? String(format: "%.2f", amount)
        return (currency == "USD" ? "US$ " : "S/. ") + number
    }

    private static func date(_ at: Int64) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "es_PE")
        formatter.timeZone = TimeZone(identifier: "America/Lima")
        formatter.dateFormat = "d MMM, HH:mm"
        return formatter.string(from: Date(timeIntervalSince1970: TimeInterval(at)))
    }
}
