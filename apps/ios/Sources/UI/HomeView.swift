import SwiftUI

/// The home: Buddy, the machine it is talking to, and what needs the user now.
struct HomeView: View {
    @Environment(AppModel.self) private var model
    @State private var chatting = false

    var body: some View {
        ScrollView {
            VStack(spacing: 20) {
                Button {
                    model.newChat()
                    chatting = true
                } label: {
                    VStack(spacing: 12) {
                        PixelSpriteView(sprite: model.sprite, state: model.answering ? "think" : model.mascotState)
                            .frame(width: 168, height: 168)
                        StatusLine(status: model.status)
                    }
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Hablar con Buddy")
                .accessibilityHint("Abre un chat nuevo")

                Button {
                    model.newChat()
                    chatting = true
                } label: {
                    Label("Escríbele a Buddy", systemImage: "square.and.pencil")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .disabled(model.status != .ready)

                ForEach(model.approvals) { approval in
                    ApprovalCard(approval: approval)
                }
                if !model.sessions.isEmpty {
                    Card(title: "Sesiones de código", symbol: "terminal") {
                        ForEach(model.sessions.prefix(5)) { session in
                            HStack {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(session.project.isEmpty ? AgentNames.name(session.agent) : session.project).font(.body.weight(.medium))
                                    Text(AgentNames.name(session.agent)).font(.caption).foregroundStyle(.secondary)
                                }
                                Spacer()
                                SessionBadge(state: session.state)
                            }
                            .accessibilityElement(children: .combine)
                        }
                    }
                }
                if !model.usage.isEmpty {
                    Card(title: "Tus planes", symbol: "gauge.with.dots.needle.50percent") {
                        ForEach(model.usage) { plan in
                            ForEach(Array(plan.windows.prefix(2).enumerated()), id: \.offset) { _, window in
                                VStack(alignment: .leading, spacing: 4) {
                                    HStack {
                                        Text("\(plan.name) · \(window.label)").font(.subheadline)
                                        Spacer()
                                        Text("\(Int(window.usedPct.rounded())) %").font(.subheadline.monospacedDigit()).foregroundStyle(.secondary)
                                    }
                                    ProgressView(value: min(max(window.usedPct, 0), 100), total: 100)
                                        .tint(window.usedPct >= 90 ? .red : window.usedPct >= 70 ? .orange : .accentColor)
                                }
                                .accessibilityElement(children: .combine)
                            }
                        }
                    }
                }
                if !model.briefing.isEmpty {
                    Card(title: "Mensajitos de hoy", symbol: "newspaper") {
                        ForEach(model.briefing.prefix(4)) { item in
                            VStack(alignment: .leading, spacing: 2) {
                                Text(item.topic).font(.caption.weight(.semibold)).foregroundStyle(.tint)
                                Text(item.text).font(.subheadline)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }
                }
            }
            .padding()
        }
        .navigationTitle(model.current?.name ?? "Buddy")
        .toolbar {
            if model.machines.count > 1 {
                ToolbarItem(placement: .topBarTrailing) {
                    Menu {
                        ForEach(model.machines) { machine in
                            Button {
                                model.select(machine)
                            } label: {
                                Label(machine.name, systemImage: machine.platform == "windows" ? "pc" : "desktopcomputer")
                            }
                        }
                    } label: {
                        Label("Cambiar de equipo", systemImage: "arrow.left.arrow.right")
                    }
                }
            }
        }
        .refreshable { await model.refresh() }
        .navigationDestination(isPresented: $chatting) { ChatView() }
        #if DEBUG
        // BUDDY_DEBUG_SEND="…": opens a chat and sends it once the machine is connected (to look at an answer
        // arriving without touching the simulator).
        .task {
            guard let text = ProcessInfo.processInfo.environment["BUDDY_DEBUG_SEND"] else { return }
            for _ in 0..<50 where model.status != .ready { try? await Task.sleep(for: .milliseconds(200)) }
            model.newChat()
            chatting = true
            await model.send(text)
        }
        #endif
    }
}

/// Whether the machine is there, in words and a dot.
struct StatusLine: View {
    let status: RemoteLink.Status

    var body: some View {
        HStack(spacing: 6) {
            Circle().fill(color).frame(width: 8, height: 8)
            Text(words).font(.subheadline).foregroundStyle(.secondary)
        }
        .accessibilityElement(children: .combine)
    }

    private var color: Color {
        switch status {
        case .ready: .green
        case .connecting: .yellow
        case .machineAway, .offline: .gray
        }
    }

    private var words: String {
        switch status {
        case .ready: "Encendido y conectado"
        case .connecting: "Conectando…"
        case .machineAway: "El equipo está apagado o dormido"
        case let .offline(reason): reason
        }
    }
}

struct SessionBadge: View {
    let state: String

    var body: some View {
        Text(words)
            .font(.caption.weight(.medium))
            .padding(.horizontal, 8).padding(.vertical, 3)
            .background(color.opacity(0.18), in: Capsule())
            .foregroundStyle(color)
    }

    private var words: String {
        switch state {
        case "working": "Trabajando"
        case "waiting": "Te espera"
        case "done": "Listo"
        case "error": "Error"
        default: state
        }
    }

    private var color: Color {
        switch state {
        case "working": .blue
        case "waiting": .orange
        case "done": .green
        case "error": .red
        default: .gray
        }
    }
}

/// A titled group on the home.
struct Card<Content: View>: View {
    let title: String
    let symbol: String
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label(title, systemImage: symbol).font(.headline)
            content
        }
        .padding()
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
    }
}

/// An agent on the machine asks for permission. The whole command is shown (it scrolls) before anything is
/// allowed; allowing asks for Face ID.
struct ApprovalCard: View {
    @Environment(AppModel.self) private var model
    let approval: Approval
    @State private var busy = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label("\(AgentNames.name(approval.agent)) pide permiso", systemImage: "hand.raised.fill")
                .font(.headline)
                .foregroundStyle(.orange)
            Text(approval.project.isEmpty ? approval.title : "\(approval.title) · \(approval.project)")
                .font(.subheadline)
            ScrollView {
                Text(approval.detail.isEmpty ? approval.summary : approval.detail)
                    .font(.footnote.monospaced())
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled)
                    .padding(10)
            }
            .frame(maxHeight: 160)
            .background(.background.tertiary, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            if !approval.canAllow {
                Text("Es demasiado largo para revisarlo aquí: respóndelo en el equipo.").font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                Button("Rechazar", role: .destructive) { answer(false) }
                    .buttonStyle(.bordered)
                Spacer()
                if approval.canAllow {
                    Button { answer(true) } label: { Label("Permitir", systemImage: "faceid") }
                        .buttonStyle(.borderedProminent)
                }
            }
            .disabled(busy)
        }
        .padding()
        .background(.orange.opacity(0.12), in: RoundedRectangle(cornerRadius: 20, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 20, style: .continuous).strokeBorder(.orange.opacity(0.35)))
    }

    private func answer(_ allow: Bool) {
        busy = true
        Task {
            await model.answer(approval, allow: allow)
            busy = false
        }
    }
}
