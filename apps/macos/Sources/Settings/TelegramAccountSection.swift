import SwiftUI

private struct AccountChat: Decodable, Identifiable {
    let id: Int64
    let title: String
    let kind: String
}
private struct AccountPost: Decodable, Identifiable {
    let chatId: Int64
    let chat: String
    let id: Int32
    let date: Int64
    let text: String
    let photos: [String]
    var key: String { "\(chatId):\(id)" }
}
private struct AccountState: Decodable {
    var apiId: Int64?
    var configured = false
    var authorized = false
    var step = "idle"
    var name: String?
    var hint: String?
    var chats: [AccountChat] = []
    var selected: [Int64] = []
    var posts: [AccountPost] = []
    var errors: [String]?
}

/// A separate connection: personal account reads only the groups the user selects; bot pairing stays intact.
struct TelegramAccountSection: View {
    let core: BuddyCore
    @State private var state = AccountState()
    @State private var editingCredentials = false
    @State private var apiId = ""
    @State private var apiHash = ""
    @State private var phone = ""
    @State private var code = ""
    @State private var password = ""
    @State private var busy = false
    @State private var error: String?
    @State private var analysis: String?

    var body: some View {
        Section {
            if !state.configured || editingCredentials {
                Text("Conecta tu cuenta para consultar los picks de los grupos y canales que elijas.")
                    .font(.caption).foregroundStyle(.secondary)
                Link("Obtener api_id y api_hash", destination: URL(string: "https://my.telegram.org/apps")!)
                TextField("api_id", text: $apiId)
                SecureField("api_hash", text: $apiHash, prompt: Text(state.configured ? "Deja vacío para conservar el guardado" : "api_hash"))
                Button("Guardar y continuar") {
                    let data = try? JSONSerialization.data(withJSONObject: ["id": Int64(apiId.trimmingCharacters(in: .whitespacesAndNewlines)) ?? 0, "hash": apiHash])
                    perform("configure", value: data.flatMap { String(data: $0, encoding: .utf8) } ?? "{}")
                }.disabled(busy || apiId.isEmpty || (!state.configured && apiHash.isEmpty))
                if editingCredentials { Button("Cancelar") { editingCredentials = false; apiId = ""; apiHash = "" }.disabled(busy) }
            } else if !state.authorized {
                Button("Editar api_id / api_hash") {
                    apiId = state.apiId.map(String.init) ?? ""
                    apiHash = ""
                    editingCredentials = true
                }.disabled(busy)
                if state.step == "password" {
                    Text("Telegram pide tu contraseña de verificación en dos pasos.").font(.caption)
                    if let hint = state.hint { Text(hint).font(.caption).foregroundStyle(.secondary) }
                    SecureField("Contraseña de Telegram", text: $password)
                    Button("Iniciar sesión") { perform("password", value: password) }.disabled(busy || password.isEmpty)
                } else if state.step == "code" {
                    Text("Escribe el código que recibiste en Telegram.").font(.caption)
                    SecureField("Código de acceso", text: $code)
                    HStack {
                        Button("Verificar código") { perform("signIn", value: code) }.disabled(busy || code.isEmpty)
                        Button("Volver a pedir código") { perform("sendCode", value: phone) }.disabled(busy || phone.isEmpty)
                    }
                } else {
                    TextField("Teléfono con código de país (+51…)", text: $phone)
                    Button("Pedir código") { perform("sendCode", value: phone) }.disabled(busy || phone.isEmpty)
                }
            } else {
                LabeledContent("Cuenta", value: state.name ?? "Conectada")
                HStack {
                    Button("Elegir grupos") { perform("chats") }.disabled(busy)
                    Spacer()
                    Button("Cerrar sesión") { perform("signOut") }.disabled(busy)
                }
                if !state.chats.isEmpty {
                    Text("Selecciona hasta 10 grupos o canales.").font(.caption).foregroundStyle(.secondary)
                    ForEach(state.chats) { chat in
                        Toggle(chat.title, isOn: Binding(get: { state.selected.contains(chat.id) }, set: { on in
                            var ids = state.selected
                            if on { ids.append(chat.id) } else { ids.removeAll { $0 == chat.id } }
                            let data = try? JSONEncoder().encode(ids)
                            perform("select", value: data.flatMap { String(data: $0, encoding: .utf8) } ?? "[]")
                        })).disabled(busy || (!state.selected.contains(chat.id) && state.selected.count >= 10))
                    }
                }
                if !state.selected.isEmpty {
                    Text("\(state.selected.count) grupos seleccionados").font(.caption).foregroundStyle(.secondary)
                    HStack {
                        Button("Consultar mensajes") { perform("fetch") }.disabled(busy)
                        Button("Analizar con PARLEY") { perform("analyze") }.disabled(busy || state.posts.isEmpty)
                    }
                }
                if let analysis {
                    DisclosureGroup("Análisis de PARLEY") {
                        Text(analysis).textSelection(.enabled)
                        Text("También está en el historial: Telegram · Grupos").font(.caption).foregroundStyle(.secondary)
                    }
                }
                if !state.posts.isEmpty {
                    DisclosureGroup("Mensajes consultados (\(state.posts.count))") {
                        ForEach(Array(state.posts.suffix(30).reversed()), id: \.key) { post in
                            VStack(alignment: .leading, spacing: 4) {
                                Text(post.chat).font(.caption.bold())
                                Text(Date(timeIntervalSince1970: TimeInterval(post.date)), format: .dateTime.day().month().hour().minute())
                                    .font(.caption).foregroundStyle(.secondary)
                                Text(post.text.isEmpty ? "Mensaje sin texto" : post.text).textSelection(.enabled)
                                if !post.photos.isEmpty { Text("\(post.photos.count) foto(s) disponibles para PARLEY").font(.caption).foregroundStyle(.secondary) }
                            }.padding(.vertical, 6)
                        }
                    }
                }
            }
            if busy { HStack { ProgressView().controlSize(.small); Text("Conectando con Telegram…").font(.caption) } }
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
            if let errors = state.errors { ForEach(errors, id: \.self) { Text($0).font(.caption).foregroundStyle(.red) } }
        } header: {
            Label("Telegram · Cuenta personal", systemImage: "person.crop.circle")
        } footer: {
            Text("Lectura de los grupos elegidos al pulsar Consultar mensajes (últimos 20 por grupo). No envía mensajes, no se une a grupos ni los marca como leídos. Los chats privados y el contenido protegido quedan fuera. Las credenciales y la sesión se guardan en el Llavero; el código y la contraseña no se guardan. Analizar con PARLEY envía los mensajes y hasta 10 fotos al proveedor del agente.")
                .font(.caption).foregroundStyle(.secondary)
        }
        .onAppear { perform("status") }
    }

    private func perform(_ action: String, value: String = "") {
        guard !busy else { return }
        busy = true
        error = nil
        Task.detached {
            let result = Result { try core.telegramAccountRequest(action: action, value: value) }
            await MainActor.run {
                busy = false
                switch result {
                case .success(let json):
                    let data = Data(json.utf8)
                    if action == "analyze" {
                        analysis = (try? JSONSerialization.jsonObject(with: data) as? [String: String])?["analysis"]
                    } else if let next = try? JSONDecoder().decode(AccountState.self, from: data) {
                        state = next
                        if action == "configure" { apiId = ""; apiHash = ""; editingCredentials = false }
                        if action == "signIn" { code = "" }
                        if action == "password" { password = "" }
                        if action == "signOut" || action == "select" || action == "fetch" { analysis = nil }
                    } else { error = "No se pudo leer la respuesta de Telegram." }
                case .failure(let failure):
                    if case let CoreError.Hooks(message) = failure { error = message }
                    else { error = String(describing: failure) }
                }
            }
        }
    }
}
