import SwiftUI

/// Settings › Conexiones › Conectores (MCP): Buddy's built-in remote MCP servers (nothing to install). Each one has a
/// switch; the ones that take a key keep it only in the Keychain. Only agents with the web use them. The core does all of it.
struct ConnectorsSection: View {
    let core: BuddyCore
    @State private var items: [ConnectorInfo] = []
    @State private var keys: [String: String] = [:]
    @State private var error: String?

    var body: some View {
        Section {
            ForEach(items, id: \.id) { item in
                VStack(alignment: .leading, spacing: 8) {
                    Toggle(isOn: binding(item)) {
                        Text(item.name)
                        Text(item.description)
                    }
                    HStack(spacing: 12) {
                        if let url = URL(string: item.site) {
                            Link(destination: url) {
                                Label("Sitio", systemImage: "arrow.up.right.square")
                            }
                            .font(.caption)
                        }
                        if item.keyOptional && item.hasKey {
                            Label("Clave guardada en tu Llavero", systemImage: "key.fill")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    if item.keyOptional {
                        HStack {
                            SecureField("Clave (opcional)", text: keyBinding(item.id), prompt: Text("Clave (opcional)"))
                            Button("Guardar") { save(item.id, keys[item.id] ?? "") }
                                .disabled((keys[item.id] ?? "").trimmingCharacters(in: .whitespaces).isEmpty)
                            if item.hasKey {
                                Button("Quitar") { save(item.id, "") }
                            }
                        }
                    }
                }
                .padding(.vertical, 2)
            }
            if let error {
                Text(error).font(.caption).foregroundStyle(.red)
            }
        } header: {
            Label("Conectores (MCP)", systemImage: "puzzlepiece.extension")
        } footer: {
            Text("Vienen con Buddy y no instalan nada. Los usan Claude, Codex y Gemini en los agentes que pueden buscar en la web. La clave solo da más uso y se guarda en tu Llavero; con Gemini van sin clave.")
                .font(.caption).foregroundStyle(.secondary)
        }
        .onAppear(perform: reload)
    }

    private func reload() {
        items = core.connectors()
    }

    private func binding(_ item: ConnectorInfo) -> Binding<Bool> {
        Binding(
            get: { items.first { $0.id == item.id }?.enabled ?? item.enabled },
            set: { on in
                do {
                    try core.setConnectorEnabled(id: item.id, on: on)
                    error = nil
                } catch {
                    self.error = Self.message(error)
                }
                reload()
            }
        )
    }

    private func keyBinding(_ id: String) -> Binding<String> {
        Binding(get: { keys[id] ?? "" }, set: { keys[id] = $0 })
    }

    private func save(_ id: String, _ key: String) {
        do {
            try core.setConnectorKey(id: id, key: key)
            keys[id] = ""
            error = nil
        } catch {
            self.error = Self.message(error)
        }
        reload()
    }

    private static func message(_ failure: Error) -> String {
        if case let CoreError.Hooks(message) = failure { return message }
        return String(describing: failure)
    }
}
