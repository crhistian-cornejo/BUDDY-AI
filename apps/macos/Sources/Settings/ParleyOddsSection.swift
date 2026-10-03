import SwiftUI

private struct ParleyOddsState: Decodable {
    var enabled: Bool
    var hasKey: Bool
    var bookmaker: String
    var dailyCap: Int
}

/// The API credential is entered here and never enters a chat or comes back from the core.
struct ParleyOddsSection: View {
    let core: BuddyCore
    @State private var status: ParleyOddsState?
    @State private var key = ""
    @State private var message: String?
    @State private var busy = false

    var body: some View {
        Section("PARLEY · Cuotas") {
            Text("PARLEY consulta OddsPapi para obtener el calendario y las cuotas de Betano Perú antes de analizar. También revisa el bot y los mensajes de hoy de tus grupos seleccionados en Telegram.")
                .font(.caption).foregroundStyle(.secondary)
            if let status {
                Text(status.hasKey && status.enabled ? "OddsPapi configurado · Betano Perú" : "Falta configurar OddsPapi")
                Text("Máximo \(status.dailyCap) consultas al día; calendario y cuotas se reutilizan durante 5 minutos.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            SecureField("Clave de OddsPapi", text: $key)
                .textContentType(.password)
            HStack {
                Button("Guardar") { request("configure", value: key) }
                    .disabled(busy || key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                if status?.hasKey == true {
                    Button("Quitar clave") { request("configure", value: "") }.disabled(busy)
                }
                Link("Obtener clave", destination: URL(string: "https://oddspapi.io/en/account")!)
            }
            if let message { Text(message).font(.caption).foregroundStyle(.secondary) }
        }
        .onAppear { request("status") }
    }

    private func request(_ action: String, value: String = "") {
        busy = true
        message = nil
        Task {
            let core = core
            let result = await Task.detached {
                Result { try core.parleyOddsRequest(action: action, value: value) }
            }.value
            busy = false
            switch result {
            case .success(let raw):
                status = raw.data(using: .utf8).flatMap { try? JSONDecoder().decode(ParleyOddsState.self, from: $0) }
                key = ""
                if action == "configure" { message = value.isEmpty ? "Clave retirada." : "Clave guardada en el llavero. Se verificará en la próxima consulta." }
            case .failure(let error):
                message = String(describing: error)
            }
        }
    }
}
