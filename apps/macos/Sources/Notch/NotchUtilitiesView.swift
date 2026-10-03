import AppKit
import SwiftUI

struct NotchShelfView: View {
    let model: NotchModel
    let actions: NotchActions

    var body: some View {
        VStack(spacing: 12) {
            HStack {
                Text("Tu bandeja").font(.system(size: 12, weight: .semibold))
                Text("\(model.dropped.count)/32").font(.system(size: 10)).foregroundStyle(.secondary)
                Spacer()
                Button("Añadir…", action: actions.addFiles).buttonStyle(IslandButtonStyle(prominent: false, compact: true))
            }.frame(height: 24)
            ScrollView {
                if model.dropped.isEmpty {
                    VStack(spacing: 8) {
                        Image(systemName: "tray").font(.system(size: 22))
                        Text("Suelta archivos aquí o pulsa Añadir.").font(.system(size: 12))
                        Text("Se conservan al cerrar el notch.").font(.system(size: 10)).foregroundStyle(.secondary)
                    }.frame(maxWidth: .infinity, minHeight: 112)
                } else {
                    VStack(spacing: 8) {
                        ForEach(model.tools?.files ?? [], id: \.path) { file in
                            HStack(spacing: 10) {
                                Button { actions.openFile(URL(fileURLWithPath: file.path)) } label: {
                                    HStack(spacing: 10) {
                                        Image(nsImage: NSWorkspace.shared.icon(forFile: file.path)).resizable().frame(width: 22, height: 22)
                                        VStack(alignment: .leading, spacing: 1) {
                                            Text(file.name).font(.system(size: 12, weight: .medium)).lineLimit(1)
                                            Text(file.available ? URL(fileURLWithPath: file.path).deletingLastPathComponent().path : "Archivo no disponible")
                                                .font(.system(size: 10)).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                                        }
                                        Spacer(minLength: 0)
                                    }
                                }.buttonStyle(.plain).disabled(!file.available).tip(file.path)
                                Button { actions.removeFile(file.path) } label: { Image(systemName: "xmark").frame(width: 24, height: 24) }
                                    .buttonStyle(.plain).accessibilityLabel("Retirar \(file.name) de la bandeja")
                                    .tip("Retirar de la bandeja; el archivo permanece en su carpeta")
                            }.padding(.horizontal, 10).frame(height: 40)
                                .background(.white.opacity(0.06), in: RoundedRectangle(cornerRadius: 10))
                        }
                    }
                }
            }.frame(height: 114)
            HStack(spacing: 8) {
                Button("Dárselo a Buddy", action: actions.giveToBuddy).buttonStyle(IslandButtonStyle(prominent: true))
                Button("Compartir…", action: actions.share).buttonStyle(IslandButtonStyle(prominent: false))
                Button("Copiar rutas", action: actions.copyPaths).buttonStyle(IslandButtonStyle(prominent: false))
                Spacer(minLength: 0)
            }.disabled(model.tools?.files.contains(where: \.available) != true).frame(height: 28)
        }.frame(height: NotchLayout.shelfHeight)
    }
}

struct NotchUtilitiesView: View {
    let model: NotchModel
    let actions: NotchActions

    var body: some View {
        VStack(spacing: 12) {
            if (model.tools?.batteryEnabled ?? true) || (model.tools?.calendarEnabled ?? true) {
                HStack(spacing: 12) {
                    if model.tools?.batteryEnabled ?? true { battery }
                    if model.tools?.calendarEnabled ?? true { calendar }
                }.frame(height: 96)
            }
            if model.tools?.clipboardEnabled ?? true { clipboard }
            if model.tools?.batteryEnabled == false && model.tools?.calendarEnabled == false && model.tools?.clipboardEnabled == false {
                Text("Elige un widget en el menú Widgets.").font(.system(size: 12)).foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, minHeight: 96)
            }
        }.frame(height: NotchLayout.utilitiesHeight(model), alignment: .top)
    }

    private var battery: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("Batería", systemImage: "battery.100percent").font(.system(size: 11, weight: .semibold)).foregroundStyle(.secondary)
            if let battery = model.battery {
                HStack(spacing: 6) {
                    Text(battery.percent.map { "\($0) %" } ?? "—").font(.system(size: 22, weight: .semibold)).monospacedDigit()
                    if battery.pluggedIn { Image(systemName: "bolt.fill").foregroundStyle(Color.accentColor) }
                }
                Text(battery.charging ? "Cargando" : battery.pluggedIn ? "Conectado a corriente" : "Usando batería")
                    .font(.system(size: 10)).foregroundStyle(.secondary)
            } else {
                Text("Equipo sin batería").font(.system(size: 12))
                Text("Datos del sistema").font(.system(size: 10)).foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
        }.notchUtilityCard()
    }

    private var calendar: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Label("Próxima cita", systemImage: "calendar").font(.system(size: 11, weight: .semibold)).foregroundStyle(.secondary)
                Spacer(minLength: 0)
                Menu {
                    Button("Elegir agenda .ics…", action: actions.pickCalendar)
                    if model.tools?.calendarPath != nil { Button("Desconectar agenda", action: actions.clearCalendar) }
                } label: { Image(systemName: "ellipsis") }.menuStyle(.borderlessButton).fixedSize().accessibilityLabel("Fuente de la agenda")
            }
            if let event = model.appointment {
                Button(action: actions.openAppointment) {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(event.title.isEmpty ? "Sin título" : event.title).font(.system(size: 12, weight: .medium)).lineLimit(1)
                        Text(Self.when(event)).font(.system(size: 10)).foregroundStyle(Color.accentColor)
                        Text(event.source).font(.system(size: 10)).foregroundStyle(.secondary).lineLimit(1)
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }.buttonStyle(.plain).tip("\(event.title) · \(Self.when(event)) · \(event.source)\(event.location.isEmpty ? "" : " · " + event.location)\nAbrir enlace o evento en tu aplicación de calendario")
            } else if model.tools?.calendarPath == nil {
                Text("Agenda local (.ics)").font(.system(size: 10)).foregroundStyle(.secondary)
                Button("Elegir agenda…", action: actions.pickCalendar).font(.system(size: 11)).buttonStyle(.plain)
            } else {
                Text(model.calendarError.isEmpty ? "Sin próximas citas" : model.calendarError).font(.system(size: 11)).lineLimit(2)
                Text(URL(fileURLWithPath: model.tools?.calendarPath ?? "").lastPathComponent)
                    .font(.system(size: 10)).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer(minLength: 0)
        }.notchUtilityCard()
    }

    private var clipboard: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Label("Portapapeles", systemImage: "doc.on.clipboard").font(.system(size: 11, weight: .semibold)).foregroundStyle(.secondary)
                Spacer()
                Button("Guardar actual", action: actions.saveClipboard).buttonStyle(IslandButtonStyle(prominent: false, compact: true))
                    .tip("Guardar el texto que tú has copiado; no se guarda nada automáticamente")
            }
            ScrollView {
                if model.tools?.clips.isEmpty ?? true {
                    Text("Guarda textos o enlaces para copiarlos o llevarlos al chat.").font(.system(size: 11)).foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, minHeight: 58, alignment: .leading)
                } else {
                    VStack(spacing: 6) {
                        ForEach(model.tools?.clips ?? [], id: \.id) { clip in
                            HStack(spacing: 8) {
                                Text(clip.text).font(.system(size: 11)).lineLimit(1).frame(maxWidth: .infinity, alignment: .leading).tip(String(clip.text.prefix(2000)))
                                clipButton("doc.on.doc", "Copiar texto") { actions.copyClip(clip) }
                                clipButton("bubble.left", "Llevar al chat sin enviarlo") { actions.giveClip(clip) }
                                clipButton("xmark", "Quitar texto guardado") { actions.removeClip(clip.id) }
                            }.frame(height: 24)
                        }
                    }
                }
            }.frame(height: 70)
        }.padding(12).frame(height: 128).background(.white.opacity(0.07), in: RoundedRectangle(cornerRadius: 16))
            .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder(.white.opacity(0.08)))
    }

    private func clipButton(_ symbol: String, _ label: String, action: @escaping () -> Void) -> some View {
        Button(action: action) { Image(systemName: symbol).font(.system(size: 11)).frame(width: 24, height: 24) }
            .buttonStyle(.plain).accessibilityLabel(label).tip(label)
    }

    static func when(_ event: Appointment) -> String {
        let date = Date(timeIntervalSince1970: Double(event.start))
        let formatter = DateFormatter(); formatter.locale = Locale(identifier: "es")
        formatter.setLocalizedDateFormatFromTemplate(event.allDay ? "EEE d MMM" : "EEE d MMM HH:mm")
        return formatter.string(from: date) + (event.allDay ? " · todo el día" : "")
    }
}

private extension View {
    func notchUtilityCard() -> some View {
        self.padding(12).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(.white.opacity(0.07), in: RoundedRectangle(cornerRadius: 16))
            .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder(.white.opacity(0.08)))
    }
}
