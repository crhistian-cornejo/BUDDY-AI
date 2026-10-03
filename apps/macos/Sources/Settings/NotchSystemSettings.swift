import SwiftUI

struct NotchSystemSettings: View {
    let system: NotchSystemMonitor
    var body: some View {
        Section("Notch · estados del sistema") {
            Toggle("Volumen y brillo", isOn: Binding(get: { system.levelsEnabled }, set: { system.set("levels", enabled: $0) }))
            Toggle("Conexiones de dispositivos", isOn: Binding(get: { system.connectionsEnabled }, set: { system.set("connections", enabled: $0) }))
            Toggle("No molestar / Concentración", isOn: Binding(get: { system.focusEnabled }, set: { system.set("focus", enabled: $0) }))
            if system.levelsEnabled {
                LabeledContent("Indicador de las teclas", value: system.replacementReady ? "En el notch" : "Pendiente de Accesibilidad")
                    .font(.caption).foregroundStyle(.secondary)
                if !system.replacementReady {
                    Button("Activar en Accesibilidad…", action: system.requestAccessibility)
                    Text("macOS requiere permitir Buddy en Accesibilidad para sustituir el indicador de volumen y brillo.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            if system.focusEnabled && !system.focusAvailable {
                Button("Permitir estado de concentración…", action: system.requestFocus)
                Text("Debe estar disponible el estado de concentración compartido por macOS.").font(.caption).foregroundStyle(.secondary)
            }
        }
    }
}
