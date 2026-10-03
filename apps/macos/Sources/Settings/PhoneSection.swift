import Combine
import SwiftUI

extension Notification.Name {
    /// Posted when the core says the phone link changed (relay reached, a phone paired, connected or forgotten).
    static let buddyRemoteChanged = Notification.Name("buddy.remoteChanged")
}

/// Settings › Conexiones › iPhone: the relay's address and key, pairing a phone by QR, forgetting it, the switch
/// for approving from the phone, and what the phone did on this Mac. The core does all of it.
struct PhoneSection: View {
    let core: BuddyCore
    @State private var status = RemoteStatus(relay: "", hasOwnerKey: false, paired: false, phone: "", connected: false, online: false, approvals: true, pairing: false, error: "")
    @State private var relay = ""
    @State private var ownerKey = ""
    @State private var offer: PairOffer?
    @State private var actions: [RemoteAction] = []
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        Section {
            if status.paired {
                LabeledContent {
                    Button("Olvidar iPhone", role: .destructive, action: forget)
                } label: {
                    Text(status.phone.isEmpty ? "iPhone emparejado" : status.phone)
                    Text(status.online ? "Conectado ahora" : status.connected ? "Emparejado; recibe avisos mientras no está conectado" : "Sin conexión con el relé")
                }
                Toggle(isOn: Binding(get: { status.approvals }, set: setApprovals)) {
                    Text("Aprobar permisos desde el iPhone")
                    Text("Apagado, el teléfono solo puede rechazar. «Permitir siempre» y ver la pantalla nunca se aprueban desde el teléfono.")
                }
                if !actions.isEmpty {
                    DisclosureGroup("Lo que hizo el iPhone en este Mac") {
                        ForEach(Array(actions.prefix(30).enumerated()), id: \.offset) { _, action in
                            LabeledContent {
                                Text(Date(timeIntervalSince1970: TimeInterval(action.at)), format: .dateTime.day().month().hour().minute())
                                    .foregroundStyle(.secondary)
                            } label: {
                                Text(verbatim: action.text)
                            }
                            .font(.caption)
                        }
                    }
                }
            } else {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Para usar Buddy desde tu iPhone:")
                    Text("1. Despliega tu relé (carpeta relay/ del proyecto) y pega aquí su dirección y su clave de dueño.")
                    Text("2. Pulsa Emparejar y escanea el código con la app de Buddy en el iPhone.")
                }
                .font(.caption)
                .foregroundStyle(.secondary)
                TextField("Dirección del relé", text: $relay, prompt: Text(verbatim: "https://buddy-relay.ejemplo.workers.dev"))
                SecureField(status.hasOwnerKey ? "Clave del relé (guardada)" : "Clave del relé", text: $ownerKey)
                if let offer, status.pairing {
                    VStack(spacing: 8) {
                        QRCodeView(size: Int(offer.size), cells: offer.cells)
                            .frame(width: 200, height: 200)
                            .accessibilityLabel("Código para emparejar el iPhone")
                        Text("Vale 5 minutos y sirve una sola vez.")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity)
                }
                HStack {
                    if let error { Text(error).font(.caption).foregroundStyle(.red) }
                    Spacer()
                    if busy { ProgressView().controlSize(.small) }
                    Button(offer != nil && status.pairing ? "Nuevo código" : "Emparejar", action: pair)
                        .disabled(busy || relay.trimmingCharacters(in: .whitespaces).isEmpty || (!status.hasOwnerKey && ownerKey.isEmpty))
                }
            }
            if !status.error.isEmpty {
                Text(status.error).font(.caption).foregroundStyle(.red)
            }
        } header: {
            Label("iPhone", systemImage: "iphone")
        } footer: {
            Text("Todo viaja cifrado de extremo a extremo: el relé solo reenvía bytes que no puede leer. Las claves se guardan solo en tu Llavero. Desde el teléfono no se cambian ajustes, claves ni carpetas.")
                .font(.caption).foregroundStyle(.secondary)
        }
        .onAppear(perform: reload)
        .onReceive(NotificationCenter.default.publisher(for: .buddyRemoteChanged).receive(on: DispatchQueue.main)) { _ in reload() }
    }

    private func reload() {
        status = core.remoteStatus()
        actions = core.remoteLog()
        if relay.isEmpty { relay = status.relay }
        if !status.pairing { offer = nil }
    }

    private func pair() {
        busy = true
        error = nil
        let (url, key) = (relay, ownerKey)
        Task.detached {
            let result = Result { () -> PairOffer in
                try core.remoteSetRelay(url: url, ownerKey: key)
                return try core.remotePair()
            }
            await MainActor.run {
                busy = false
                switch result {
                case let .success(made):
                    offer = made
                    ownerKey = ""
                case let .failure(failure):
                    error = Self.words(failure)
                }
                reload()
            }
        }
    }

    private func forget() {
        busy = true
        Task.detached {
            core.remoteForget()
            await MainActor.run {
                busy = false
                reload()
            }
        }
    }

    private func setApprovals(_ on: Bool) {
        try? core.remoteSetApprovals(on: on)
        reload()
    }

    /// The core's own words, without the error's case name.
    private static func words(_ error: Error) -> String {
        if case let CoreError.Io(message) = error { return message }
        return error.localizedDescription
    }
}

/// A QR code painted from the core's matrix (`size` by `size`, row by row; true is dark), always dark on white with
/// its quiet margin so a camera reads it in either appearance.
struct QRCodeView: View {
    let size: Int
    let cells: [Bool]

    var body: some View {
        Canvas { context, canvas in
            let quiet = 4
            let module = min(canvas.width, canvas.height) / CGFloat(size + quiet * 2)
            context.fill(Path(CGRect(origin: .zero, size: canvas)), with: .color(.white))
            guard size > 0, cells.count == size * size else { return }
            var dark = Path()
            for row in 0..<size {
                for column in 0..<size where cells[row * size + column] {
                    dark.addRect(CGRect(x: CGFloat(column + quiet) * module, y: CGFloat(row + quiet) * module, width: module, height: module))
                }
            }
            context.fill(dark, with: .color(.black))
        }
        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
    }
}
