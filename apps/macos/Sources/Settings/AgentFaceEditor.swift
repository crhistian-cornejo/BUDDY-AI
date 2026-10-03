import AppKit
import SwiftUI

/// Settings › Agentes: an agent's face («cara») — hood colour, accessory, eyes and the accessory's colour — with a
/// live preview at 4×. The core draws it (`agentSprite`) and keeps it (`agent.<id>.cara`); this only chooses.
/// Twin of the face editor in apps/windows/src/settings/agents.ts.
struct AgentFaceEditor: View {
    let core: BuddyCore
    let agent: Agent
    @State private var options: LookOptions?
    @State private var look: AgentLook?
    @State private var preview: CGImage?

    /// The face is 34 pixels square; each one is 4 points here.
    private let previewSide: CGFloat = 34 * 4

    var body: some View {
        LabeledContent("Cara") {
            if let look, let options {
                HStack(alignment: .top, spacing: 14) {
                    previewBox
                    VStack(alignment: .leading, spacing: 10) {
                        Swatches(colors: options.colors, selected: look.color) { color in
                            change { $0.color = color }
                        }
                        Picker("Accesorio", selection: binding(\.accessory)) {
                            ForEach(options.accessories, id: \.id) { option in
                                Label(option.label, systemImage: Self.symbol(option.id)).tag(option.id)
                            }
                        }
                        .pickerStyle(.menu)
                        .labelsHidden()
                        .fixedSize()
                        .help("Lo que lleva en la cabeza o la cara")
                        Picker("Ojos", selection: binding(\.eyes)) {
                            ForEach(options.eyes, id: \.id) { Text($0.label).tag($0.id) }
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                        .fixedSize()
                        .help("Cómo mira")
                        if look.accessory != "ninguno" {
                            HStack(spacing: 8) {
                                Text("Color del accesorio").font(.caption).foregroundStyle(.secondary)
                                Button {
                                    change { $0.badge = nil }
                                } label: {
                                    Image(systemName: look.badge == nil ? "wand.and.stars.inverse" : "wand.and.stars")
                                }
                                .buttonStyle(.borderless)
                                .help("Automático: uno que resalte sobre la capucha")
                                Swatches(colors: options.colors, selected: look.badge ?? "", small: true) { color in
                                    change { $0.badge = color }
                                }
                            }
                        }
                        Button("Restablecer", systemImage: "arrow.counterclockwise") {
                            try? core.resetAgentLook(agentId: agent.id)
                            reload()
                        }
                        .buttonStyle(.borderless)
                        .font(.caption)
                        .help("Vuelve a la cara de su archivo agent.md")
                    }
                }
            } else {
                ProgressView().controlSize(.small)
            }
        }
        .onAppear(perform: reload)
    }

    private var previewBox: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(.quaternary.opacity(0.6))
            if let preview {
                Image(decorative: preview, scale: 1)
                    .interpolation(.none)
                    .resizable()
                    .frame(width: previewSide, height: previewSide)
            }
        }
        .frame(width: previewSide + 12, height: previewSide + 12)
        .accessibilityLabel("Cara de \(agent.name)")
    }

    private func binding(_ field: WritableKeyPath<AgentLook, String>) -> Binding<String> {
        Binding(get: { look?[keyPath: field] ?? "" }, set: { value in change { $0[keyPath: field] = value } })
    }

    private func change(_ edit: (inout AgentLook) -> Void) {
        guard var next = look else { return }
        edit(&next)
        look = next
        try? core.setAgentLook(agentId: agent.id, look: next)
        reload()
    }

    private func reload() {
        if options == nil { options = core.lookOptions() }
        look = core.agentLook(agentId: agent.id)
        Avatar.invalidate(agentId: agent.id)
        preview = Avatar.face(agentId: agent.id)
    }

    static func symbol(_ accessory: String) -> String {
        switch accessory {
        case "gorra": "baseball"
        case "lentes": "eyeglasses"
        case "audifonos": "headphones"
        case "corona": "crown"
        case "bandana": "wind"
        case "gorro": "snowflake"
        default: "circle.slash"
        }
    }
}

/// The palette as round swatches; the chosen one has a ring and a check.
private struct Swatches: View {
    let colors: [LookOption]
    let selected: String
    var small = false
    let onPick: (String) -> Void

    var body: some View {
        HStack(spacing: small ? 4 : 6) {
            ForEach(colors, id: \.id) { option in
                let side: CGFloat = small ? 14 : 20
                Button {
                    onPick(option.id)
                } label: {
                    Circle()
                        .fill(Color(hex: option.hex ?? "#888888"))
                        .overlay(Circle().strokeBorder(.black.opacity(0.18), lineWidth: 1))
                        .overlay {
                            if option.id == selected {
                                Image(systemName: "checkmark")
                                    .font(.system(size: side * 0.5, weight: .bold))
                                    .foregroundStyle(.black.opacity(0.65))
                            }
                        }
                        .padding(2)
                        .overlay(Circle().strokeBorder(option.id == selected ? Color.accentColor : .clear, lineWidth: 2))
                        .frame(width: side + 4, height: side + 4)
                }
                .buttonStyle(.plain)
                .help(option.label)
                .accessibilityLabel(option.label)
                .accessibilityAddTraits(option.id == selected ? .isSelected : [])
            }
        }
    }
}
