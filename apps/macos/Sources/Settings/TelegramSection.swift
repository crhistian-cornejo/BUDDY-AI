import Combine
import SwiftUI

extension Notification.Name {
    /// Posted when the core says Telegram's connection or pairing changed.
    static let buddyTelegramChanged = Notification.Name("buddy.telegramChanged")
}

/// Settings › Conexiones › Telegram: the user's own bot (token only in the Keychain), paired with one chat by a
/// `/start <code>`. PARLEY answers that chat; nothing else is heard. The core does all of it.
struct TelegramSection: View {
    let core: BuddyCore
    @State private var status = TelegramStatus(connected: false, paired: false, botName: "", pairingCode: "", error: "")
    @State private var token = ""
    @State private var busy = false
    @State private var error: String?
    @State private var note: String?

    var body: some View {
        Section {
            if !status.connected {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Para hablar con PARLEY desde Telegram y recibir ahí sus picks:")
                    Text(verbatim: "1. Abre @BotFather, envíale /newbot y elige un nombre para tu bot.")
                    Text("2. Copia el token que te da y pégalo aquí.")
                }
                .font(.caption)
                .foregroundStyle(.secondary)
                Link(destination: URL(string: "https://t.me/BotFather")!) {
                    Label("Abrir @BotFather", systemImage: "arrow.up.right.square")
                }
                SecureField("Token del bot", text: $token, prompt: Text(verbatim: "123456789:AA…"))
                HStack {
                    if let error { Text(error).font(.caption).foregroundStyle(.red) }
                    Spacer()
                    if busy { ProgressView().controlSize(.small) }
                    Button("Conectar", action: connect)
                        .disabled(busy || token.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            } else if !status.paired {
                LabeledContent {
                    Button("Desconectar", action: disconnect)
                } label: {
                    Text("Conectado con \(status.botName)")
                    Text("Falta vincular tu chat")
                }
                VStack(alignment: .leading, spacing: 6) {
                    Text("Envía este mensaje a \(status.botName) desde tu Telegram:")
                        .font(.caption).foregroundStyle(.secondary)
                    Text(verbatim: "/start \(status.pairingCode)")
                        .font(.system(.title2, design: .monospaced).weight(.semibold))
                        .textSelection(.enabled)
                    Text("El código vale 15 minutos. Solo el chat que lo envíe podrá hablar con Buddy.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                HStack {
                    if let link = startLink {
                        Link(destination: link) {
                            Label("Abrir \(status.botName) en Telegram", systemImage: "paperplane")
                        }
                    }
                    Spacer()
                    Button("Nuevo código", action: newCode)
                }
            } else {
                LabeledContent {
                    HStack {
                        Button("Enviar prueba", action: sendTest).disabled(busy)
                        Button("Desconectar", action: disconnect)
                    }
                } label: {
                    Text("Conectado con \(status.botName)")
                    Text("Tu chat está vinculado: lo que le escribas lo responde PARLEY.")
                }
                HStack {
                    if let note { Text(note).font(.caption).foregroundStyle(.secondary) }
                    Spacer()
                    Button("Cambiar de chat", action: newCode)
                }
            }
            if !status.error.isEmpty {
                Text(status.error).font(.caption).foregroundStyle(.red)
            }
        } header: {
            Label("Telegram", systemImage: "paperplane")
        } footer: {
            Text("El token se guarda solo en tu Llavero. Buddy atiende únicamente al chat vinculado, y desde Telegram PARLEY no ejecuta comandos ni cambia archivos.")
                .font(.caption).foregroundStyle(.secondary)
        }
        .onAppear(perform: reload)
        .onReceive(NotificationCenter.default.publisher(for: .buddyTelegramChanged).receive(on: DispatchQueue.main)) { _ in reload() }
    }

    /// t.me/<bot>?start=<code>: Telegram opens the bot with «/start <code>» ready to send.
    private var startLink: URL? {
        let bot = status.botName.hasPrefix("@") ? String(status.botName.dropFirst()) : status.botName
        guard !bot.isEmpty, !status.pairingCode.isEmpty else { return nil }
        return URL(string: "https://t.me/\(bot)?start=\(status.pairingCode)")
    }

    private func reload() {
        status = core.telegramStatus()
        #if DEBUG
        // BUDDY_DEBUG_TELEGRAM=pairing|paired shows those states without a real bot (to look at them).
        switch ProcessInfo.processInfo.environment["BUDDY_DEBUG_TELEGRAM"] {
        case "pairing": status = TelegramStatus(connected: true, paired: false, botName: "@mi_buddy_bot", pairingCode: "482913", error: "")
        case "paired": status = TelegramStatus(connected: true, paired: true, botName: "@mi_buddy_bot", pairingCode: "", error: "")
        default: break
        }
        #endif
    }

    private func connect() {
        busy = true
        error = nil
        let value = token
        Task.detached {
            let result = Result { try core.telegramConnect(token: value) }
            await MainActor.run {
                busy = false
                switch result {
                case .success:
                    token = ""
                    reload()
                case let .failure(failure):
                    error = Self.message(failure)
                }
            }
        }
    }

    private func disconnect() {
        try? core.telegramDisconnect()
        note = nil
        reload()
    }

    private func newCode() {
        _ = try? core.telegramNewPairingCode()
        note = nil
        reload()
    }

    private func sendTest() {
        busy = true
        note = nil
        Task.detached {
            let result = Result { try core.telegramSend(text: "¡Hola! Soy Buddy. Así te llegarán los mensajes y los picks de PARLEY.") }
            await MainActor.run {
                busy = false
                switch result {
                case .success: note = "Enviado. Míralo en Telegram."
                case let .failure(failure): note = Self.message(failure)
                }
            }
        }
    }

    private static func message(_ failure: Error) -> String {
        if case let CoreError.Hooks(message) = failure { return message }
        return String(describing: failure)
    }
}
