import AppKit
import SwiftUI

/// Sizes shared by the chat views (an 4-pt grid, SF Symbols at one size).
enum ChatMetrics {
    static let symbol: CGFloat = 13
    static let button: CGFloat = 28
    static let composerHeight: CGFloat = 44
    static let composerWidth: CGFloat = 340
    static let chatWidth: CGFloat = 420
    static let headerHeight: CGFloat = 44
    static let cornerRadius: CGFloat = 18
}

/// The compact composer next to Buddy: the field and send (stop while an answer is written). It sits on the
/// system's glass, so it follows light and dark mode by itself.
struct ComposerView: View {
    @Bindable var chat: ChatController
    var onClose: () -> Void
    @FocusState private var focused: Bool

    private var empty: Bool { chat.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }

    var body: some View {
        HStack(alignment: .bottom, spacing: 8) {
            TextField("Pregúntale a Buddy", text: $chat.draft, axis: .vertical)
                .textFieldStyle(.plain)
                .font(.body)
                .lineLimit(1...6)
                .focused($focused)
                .onSubmit { chat.send() }
                .padding(.vertical, 5)
            Group {
                if chat.streaming {
                    Button(action: chat.stop) {
                        Image(systemName: "stop.fill")
                            .font(.system(size: 10, weight: .bold))
                            .frame(width: 16, height: 16)
                    }
                    .tip("Detener la respuesta")
                } else {
                    Button(action: chat.send) {
                        Image(systemName: "arrow.up")
                            .font(.system(size: 12, weight: .bold))
                            .frame(width: 16, height: 16)
                    }
                    .disabled(empty)
                    .tip("Enviar (↩)")
                }
            }
            .buttonStyle(.borderedProminent)
            .buttonBorderShape(.circle)
            .controlSize(.small)
        }
        .padding(.leading, 16)
        .padding(.trailing, 8)
        .padding(.vertical, 8)
        .frame(width: ChatMetrics.composerWidth)
        .onAppear { focused = true }
        .onExitCommand(perform: onClose)
    }
}

/// The chat above the composer once there is an answer: a header with the usual actions and the messages.
struct ChatView: View {
    @Bindable var chat: ChatController
    var onClose: () -> Void
    var onHistory: () -> Void
    var onHeight: (CGFloat) -> Void

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    ForEach(chat.messages) { message in
                        MessageRow(message: message,
                                   isLastAnswer: message.id == chat.messages.last(where: { $0.role == "assistant" })?.id,
                                   canRegenerate: !chat.streaming,
                                   onRegenerate: chat.regenerate)
                    }
                }
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { onHeight($0 + ChatMetrics.headerHeight + 1) }
            }
            // New text keeps the end in view, without jumping the scroll by hand.
            .defaultScrollAnchor(.bottom)
            .scrollIndicators(.automatic)
        }
        .frame(width: ChatMetrics.chatWidth)
        .frame(maxHeight: .infinity, alignment: .top)
        .onAppear { chat.refreshRecent() }
        .onChange(of: chat.streaming) { _, _ in chat.refreshRecent() }
        .onExitCommand(perform: onClose)
    }

    private var header: some View {
        HStack(spacing: 4) {
            Text(chat.title)
                .font(.headline)
                .lineLimit(1)
                .truncationMode(.tail)
                .padding(.leading, 4)
            Spacer(minLength: 8)
            HeaderButton(symbol: "square.and.pencil", help: "Nuevo chat (⌘N)", action: chat.newChat)
                .keyboardShortcut("n", modifiers: .command)
            HeaderButton(symbol: "clock.arrow.circlepath", help: "Buscar en el historial (⌘F)", action: onHistory)
                .keyboardShortcut("f", modifiers: .command)
            HeaderButton(symbol: "xmark", help: "Cerrar (esc)", action: onClose)
        }
        .padding(.horizontal, 8)
        .frame(height: ChatMetrics.headerHeight)
    }
}

/// An SF Symbol button of the header: one size, one hit area, a tooltip.
private struct HeaderButton: View {
    let symbol: String
    let help: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: ChatMetrics.symbol, weight: .medium))
                .frame(width: ChatMetrics.button, height: ChatMetrics.button)
                .contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .foregroundStyle(.secondary)
        .tip(help)
    }
}

private struct MessageRow: View {
    let message: LiveMessage
    let isLastAnswer: Bool
    let canRegenerate: Bool
    let onRegenerate: () -> Void
    @State private var hovering = false

    var body: some View {
        if message.role == "user" {
            HStack {
                Spacer(minLength: 48)
                Text(message.content)
                    .font(.body)
                    .textSelection(.enabled)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .background(.quaternary, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            }
        } else {
            VStack(alignment: .leading, spacing: 6) {
                AuthorLine(name: message.author ?? "Buddy", provider: message.provider)
                if let activity = message.activity {
                    ActivityLine(activity: activity)
                }
                if message.failed {
                    Label(message.content, systemImage: "exclamationmark.triangle.fill")
                        .font(.body)
                        .foregroundStyle(.red)
                } else if !message.content.isEmpty || !message.sources.isEmpty {
                    AssistantBubble(message: message)
                }
                if !message.isStreaming && !message.content.isEmpty {
                    MessageActions(text: message.content, canRegenerate: isLastAnswer && canRegenerate,
                                   onRegenerate: onRegenerate)
                        .opacity(isLastAnswer || hovering ? 1 : 0)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
            .onHover { hovering = $0 }
        }
    }
}

/// Buddy's face, the agent's name and the mark of the service that wrote it.
private struct AuthorLine: View {
    let name: String
    let provider: String?

    var body: some View {
        HStack(spacing: 6) {
            AvatarView(size: 18)
                .tip(name == "Buddy" ? "Buddy" : "\(name), del equipo de Buddy")
            Text(name)
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
            ProviderMark(provider: provider)
        }
    }
}

/// Copy and write again, under an answer, like the usual chat apps.
private struct MessageActions: View {
    let text: String
    let canRegenerate: Bool
    let onRegenerate: () -> Void
    @State private var copied = false

    var body: some View {
        HStack(spacing: 2) {
            ActionButton(symbol: copied ? "checkmark" : "doc.on.doc", help: copied ? "Copiado" : "Copiar respuesta") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(text, forType: .string)
                copied = true
                Task { try? await Task.sleep(for: .seconds(1.5)); copied = false }
            }
            if canRegenerate {
                ActionButton(symbol: "arrow.clockwise", help: "Rehacer la respuesta", action: onRegenerate)
            }
        }
        .padding(.leading, -6)
    }
}

private struct ActionButton: View {
    let symbol: String
    let help: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 12, weight: .medium))
                .contentTransition(.symbolEffect(.replace))
                .frame(width: 24, height: 24)
                .contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .foregroundStyle(.secondary)
        .tip(help)
        .accessibilityLabel(help)
    }
}

/// What the agent is doing, with a symbol that moves: thinking, searching the web, reading a page, handing the
/// request to a specialist.
struct ActivityLine: View {
    let activity: ChatActivity
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: activity.symbol)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.tint)
                .symbolEffect(.pulse, options: .repeating, isActive: !reduceMotion)
                .frame(width: 16)
            AnswerStatusView(text: activity.text)
        }
        .transition(.opacity)
    }
}


/// A short line from Buddy next to the mascot (the hello).
struct BubbleView: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.callout.weight(.medium))
            .padding(.horizontal, 14)
            .padding(.vertical, 9)
            .fixedSize()
    }
}
