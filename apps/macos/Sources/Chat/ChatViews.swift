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
                    .help("Detener la respuesta")
                } else {
                    Button(action: chat.send) {
                        Image(systemName: "arrow.up")
                            .font(.system(size: 12, weight: .bold))
                            .frame(width: 16, height: 16)
                    }
                    .disabled(empty)
                    .help("Enviar (↩)")
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
    var onHeight: (CGFloat) -> Void

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 16) {
                        ForEach(chat.messages) { message in
                            MessageRow(message: message).id(message.id)
                        }
                    }
                    .padding(16)
                    .background(GeometryReader { g in Color.clear.preference(key: HeightKey.self, value: g.size.height) })
                }
                .scrollIndicators(.automatic)
                .onChange(of: chat.messages.last?.content) { _, _ in
                    if let last = chat.messages.last { proxy.scrollTo(last.id, anchor: .bottom) }
                }
            }
        }
        .frame(width: ChatMetrics.chatWidth)
        .frame(maxHeight: .infinity, alignment: .top)
        .onPreferenceChange(HeightKey.self) { h in onHeight(h + ChatMetrics.headerHeight + 1) }
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
            HeaderButton(symbol: "square.and.pencil", help: "Nuevo chat", action: chat.newChat)
            Menu {
                if chat.recent.isEmpty {
                    Text("Sin chats todavía")
                } else {
                    ForEach(chat.recent, id: \.id) { (summary: ChatSummary) in
                        Button(summary.title) { chat.open(summary.id) }
                    }
                }
            } label: {
                Image(systemName: "clock.arrow.circlepath")
                    .font(.system(size: ChatMetrics.symbol, weight: .medium))
                    .frame(width: ChatMetrics.button, height: ChatMetrics.button)
                    .contentShape(Rectangle())
            }
            .menuStyle(.button)
            .buttonStyle(.borderless)
            .menuIndicator(.hidden)
            .fixedSize()
            .foregroundStyle(.secondary)
            .help("Chats recientes")
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
        .help(help)
    }
}

private struct MessageRow: View {
    let message: LiveMessage

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
                if let author = message.author {
                    Label(author, systemImage: message.author?.hasPrefix("Buddy") == true ? "sparkle" : "person.crop.circle.badge.checkmark")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .labelStyle(.titleAndIcon)
                }
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
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contextMenu {
                Button("Copiar respuesta") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(message.content, forType: .string)
                }
            }
        }
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

private struct HeightKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
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
