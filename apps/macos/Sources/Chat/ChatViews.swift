import AppKit
import SwiftUI

/// The compact composer next to Buddy: «+», the field, «Nuevo chat», the microphone and send (or stop).
struct ComposerView: View {
    @Bindable var chat: ChatController
    let tokens: DesignTokens
    var onClose: () -> Void
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 6) {
            icon("plus", help: "Adjuntar (pronto)") {}
                .disabled(true)
            TextField("Pregúntale a Buddy…", text: $chat.draft, axis: .vertical)
                .textFieldStyle(.plain)
                .font(.system(size: tokens.font.sizeBody))
                .foregroundStyle(Color(hex: tokens.color.text))
                .lineLimit(1...5)
                .focused($focused)
                .onSubmit { chat.send() }
            icon("square.and.pencil", help: "Iniciar nuevo chat") { chat.newChat() }
            icon("mic", help: "Micrófono (pronto)") {}
                .disabled(true)
            if chat.streaming {
                round("stop.fill", help: "Detener") { chat.stop() }
            } else {
                round("arrow.up", help: "Enviar") { chat.send() }
                    .disabled(chat.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(Panel(tokens: tokens, radius: tokens.radius.card))
        .onAppear { focused = true }
        .onExitCommand(perform: onClose)
    }

    private func icon(_ name: String, help: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: name)
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(Color(hex: tokens.color.textMuted))
                .frame(width: 26, height: 26)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(help)
    }

    private func round(_ name: String, help: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: name)
                .font(.system(size: 12, weight: .bold))
                .foregroundStyle(Color.black)
                .frame(width: 26, height: 26)
                .background(Circle().fill(Color(hex: tokens.color.text)))
        }
        .buttonStyle(.plain)
        .help(help)
        .keyboardShortcut(.return, modifiers: .command)
    }
}

/// The chat that grows above the composer once there is an answer.
struct ChatView: View {
    @Bindable var chat: ChatController
    let tokens: DesignTokens
    var onClose: () -> Void
    var onHeight: (CGFloat) -> Void

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider().overlay(Color(hex: tokens.color.stroke))
            ScrollViewReader { proxy in
                ScrollView {
                    VStack(alignment: .leading, spacing: 14) {
                        ForEach(chat.messages) { message in
                            row(message).id(message.id)
                        }
                    }
                    .padding(14)
                    .background(GeometryReader { g in Color.clear.preference(key: HeightKey.self, value: g.size.height) })
                }
                .onChange(of: chat.messages.last?.content) { _, _ in
                    if let last = chat.messages.last { proxy.scrollTo(last.id, anchor: .bottom) }
                }
            }
        }
        .background(Panel(tokens: tokens, radius: tokens.radius.card))
        .onPreferenceChange(HeightKey.self) { h in onHeight(h + 44) }
        .onAppear { chat.refreshRecent() }
        .onChange(of: chat.streaming) { _, _ in chat.refreshRecent() }
        .onExitCommand(perform: onClose)
    }

    private var header: some View {
        HStack(spacing: 8) {
            Text("Buddy")
                .font(.system(size: tokens.font.sizeTitle, weight: .semibold))
                .foregroundStyle(Color(hex: tokens.color.text))
            Spacer()
            Menu {
                recentItems
            } label: {
                Image(systemName: "clock.arrow.circlepath")
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .help("Chats recientes")
            Button(action: onClose) { Image(systemName: "xmark") }
                .buttonStyle(.plain)
                .help("Cerrar (Esc)")
        }
        .font(.system(size: 12, weight: .medium))
        .foregroundStyle(Color(hex: tokens.color.textMuted))
        .padding(.horizontal, 14)
        .frame(height: 43)
    }

    @ViewBuilder
    private var recentItems: some View {
        if chat.recent.isEmpty {
            Text("Sin chats todavía")
        } else {
            ForEach(chat.recent, id: \.id) { (summary: ChatSummary) in
                Button(summary.title) { chat.open(summary.id) }
            }
        }
    }

    @ViewBuilder
    private func row(_ message: LiveMessage) -> some View {
        if message.role == "user" {
            HStack {
                Spacer(minLength: 40)
                Text(message.content)
                    .font(.system(size: tokens.font.sizeBody))
                    .foregroundStyle(Color(hex: tokens.color.text))
                    .textSelection(.enabled)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .background(RoundedRectangle(cornerRadius: tokens.radius.bubble, style: .continuous)
                        .fill(Color(hex: tokens.color.surfaceRaised)))
            }
        } else {
            VStack(alignment: .leading, spacing: 6) {
                if let author = message.author {
                    Text(author)
                        .font(.system(size: tokens.font.sizeSmall, weight: .semibold))
                        .foregroundStyle(Color(hex: tokens.color.accent))
                }
                if message.failed {
                    Text(message.content)
                        .font(.system(size: tokens.font.sizeBody))
                        .foregroundStyle(Color(hex: tokens.color.danger))
                } else {
                    AssistantBubble(message: message)
                }
            }
            .contextMenu {
                Button("Copiar respuesta") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(message.content, forType: .string)
                }
            }
        }
    }
}

private struct HeightKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}

/// The dark rounded panel both chat windows sit on.
struct Panel: View {
    let tokens: DesignTokens
    let radius: Double

    var body: some View {
        RoundedRectangle(cornerRadius: radius, style: .continuous)
            .fill(Color(hex: tokens.color.surface))
            .overlay(RoundedRectangle(cornerRadius: radius, style: .continuous)
                .strokeBorder(Color(hex: tokens.color.stroke), lineWidth: 1))
            .shadow(color: .black.opacity(0.35), radius: 14, y: 6)
    }
}
