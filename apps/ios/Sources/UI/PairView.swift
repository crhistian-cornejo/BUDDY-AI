import AVFoundation
import SwiftUI

/// Pairing with a computer: scan the code Buddy shows in Ajustes › Conexiones › iPhone (or paste its link).
struct PairView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    var first = false
    @State private var link = ""

    var body: some View {
        ScrollView {
            VStack(spacing: 20) {
                if first {
                    PixelSpriteView(sprite: nil).frame(width: 96, height: 96)
                    Text("Buddy en tu iPhone").font(.title.weight(.bold))
                    Text("Empareja este iPhone con tu Mac o tu PC para hablar con Buddy, ver tus sesiones y aprobar permisos desde donde estés.")
                        .multilineTextAlignment(.center)
                        .foregroundStyle(.secondary)
                }
                VStack(alignment: .leading, spacing: 8) {
                    Label("En tu equipo, abre Buddy › Ajustes › Conexiones › iPhone y pulsa Emparejar.", systemImage: "1.circle.fill")
                    Label("Apunta la cámara al código que aparece.", systemImage: "2.circle.fill")
                }
                .font(.subheadline)
                .frame(maxWidth: .infinity, alignment: .leading)

                #if targetEnvironment(simulator)
                Text("El simulador no tiene cámara: pega el enlace del código.").font(.footnote).foregroundStyle(.secondary)
                #else
                QRScanner { found in
                    guard !model.pairing else { return }
                    Task { await pair(found) }
                }
                .frame(height: 280)
                .clipShape(RoundedRectangle(cornerRadius: 24, style: .continuous))
                .accessibilityLabel("Cámara para leer el código de emparejado")
                #endif

                TextField("buddy://pair?d=…", text: $link, axis: .vertical)
                    .lineLimit(1...3)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .textFieldStyle(.roundedBorder)
                    .accessibilityLabel("Enlace del código")
                Button {
                    Task { await pair(link) }
                } label: {
                    if model.pairing { ProgressView() } else { Text("Emparejar con el enlace").frame(maxWidth: .infinity) }
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .disabled(model.pairing || !link.hasPrefix("buddy://pair"))

                if let error = model.pairError {
                    Label(error, systemImage: "exclamationmark.triangle").font(.footnote).foregroundStyle(.red)
                }
                Text("La conexión va cifrada de extremo a extremo: el relé por el que pasa no puede leerla.")
                    .font(.footnote).foregroundStyle(.secondary).multilineTextAlignment(.center)
            }
            .padding()
        }
        .navigationTitle(first ? "" : "Añadir equipo")
        .navigationBarTitleDisplayMode(.inline)
    }

    private func pair(_ uri: String) async {
        let before = model.machines.count
        await model.pair(uri)
        if model.machines.count > before, !first { dismiss() }
    }
}

/// The paired computers: switch, add, forget.
struct MachinesView: View {
    @Environment(AppModel.self) private var model
    @State private var adding = false

    var body: some View {
        List {
            Section {
                ForEach(model.machines) { machine in
                    Button {
                        model.select(machine)
                    } label: {
                        HStack {
                            Image(systemName: machine.platform == "windows" ? "pc" : "desktopcomputer").frame(width: 28)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(machine.name).font(.body.weight(.medium))
                                if machine.id == model.current?.id { StatusLine(status: model.status) }
                            }
                            Spacer()
                            if machine.id == model.current?.id { Image(systemName: "checkmark").foregroundStyle(.tint) }
                        }
                    }
                    .foregroundStyle(.primary)
                    .swipeActions {
                        Button("Olvidar", role: .destructive) { model.forget(machine) }
                    }
                }
            } footer: {
                Text("Cada equipo guarda su propio historial. Desliza para olvidar uno: sus claves se borran de este iPhone.")
            }
            Section {
                Button { adding = true } label: { Label("Añadir otro equipo", systemImage: "qrcode.viewfinder") }
            }
        }
        .navigationTitle("Equipos")
        .navigationDestination(isPresented: $adding) { PairView() }
    }
}

/// The camera, reading QR codes only. It reports each `buddy://pair` link it sees.
struct QRScanner: UIViewRepresentable {
    let onFound: (String) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(onFound: onFound) }

    func makeUIView(context: Context) -> Preview {
        let view = Preview()
        view.backgroundColor = .black
        let session = AVCaptureSession()
        guard let camera = AVCaptureDevice.default(for: .video), let input = try? AVCaptureDeviceInput(device: camera), session.canAddInput(input) else { return view }
        session.addInput(input)
        let output = AVCaptureMetadataOutput()
        guard session.canAddOutput(output) else { return view }
        session.addOutput(output)
        output.setMetadataObjectsDelegate(context.coordinator, queue: .main)
        output.metadataObjectTypes = [.qr]
        view.layer.session = session
        view.layer.videoGravity = .resizeAspectFill
        context.coordinator.session = session
        // Starting the camera blocks: off the main thread.
        let running = Running(session: session)
        DispatchQueue.global(qos: .userInitiated).async { running.session.startRunning() }
        return view
    }

    func updateUIView(_ view: Preview, context: Context) {}

    static func dismantleUIView(_ view: Preview, coordinator: Coordinator) {
        guard let session = coordinator.session else { return }
        let running = Running(session: session)
        DispatchQueue.global(qos: .userInitiated).async { running.session.stopRunning() }
    }

    /// The capture session, handed to the queue that starts and stops it (AVFoundation allows that from any thread).
    private struct Running: @unchecked Sendable {
        let session: AVCaptureSession
    }

    final class Preview: UIView {
        override class var layerClass: AnyClass { AVCaptureVideoPreviewLayer.self }
        override var layer: AVCaptureVideoPreviewLayer { super.layer as! AVCaptureVideoPreviewLayer }
    }

    @MainActor
    final class Coordinator: NSObject, AVCaptureMetadataOutputObjectsDelegate {
        let onFound: (String) -> Void
        var session: AVCaptureSession?
        private var last = ""

        init(onFound: @escaping (String) -> Void) { self.onFound = onFound }

        nonisolated func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput objects: [AVMetadataObject], from connection: AVCaptureConnection) {
            guard let text = (objects.first as? AVMetadataMachineReadableCodeObject)?.stringValue, text.hasPrefix("buddy://pair") else { return }
            Task { @MainActor in self.found(text) }
        }

        /// The same code is in view many times a second: it is reported once.
        private func found(_ text: String) {
            guard text != last else { return }
            last = text
            onFound(text)
        }
    }
}
