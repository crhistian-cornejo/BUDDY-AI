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
    @State private var pasteMonitor = MonitorBox()

    private var empty: Bool { chat.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && chat.attachments.isEmpty }

    var body: some View {
        VStack(spacing: -10) {
            if !chat.queued.isEmpty { queue }
            VStack(alignment: .leading, spacing: 8) {
                if let error = chat.queueError {
                    Text(error).font(.caption).foregroundStyle(.red).padding(.horizontal, 8)
                }
                if !chat.attachments.isEmpty {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 6) {
                            ForEach(chat.attachments, id: \.self) { url in
                                FileChip(path: url.path) { chat.detach(url) }
                            }
                        }
                    }
                }
                field
            }
            .padding(8)
            .frame(width: ChatMetrics.composerWidth)
            .buddySurface(cornerRadius: ChatMetrics.composerHeight / 2, prominent: true, margin: 0)
            .overlay {
                RoundedRectangle(cornerRadius: ChatMetrics.composerHeight / 2, style: .continuous)
                    .strokeBorder(Color.accentColor.opacity(focused ? 0.65 : 0), lineWidth: 1.5)
                    .allowsHitTesting(false)
            }
        }
        .frame(width: ChatMetrics.composerWidth)
        .onAppear {
            focused = true
            installPasteMonitor()
        }
        .onDisappear { removePasteMonitor() }
        .onExitCommand(perform: onClose)
        .onDrop(of: [.fileURL], isTargeted: nil) { providers in
            for provider in providers {
                _ = provider.loadObject(ofClass: URL.self) { url, _ in
                    guard let url else { return }
                    Task { @MainActor in chat.attach([url]) }
                }
            }
            return true
        }
    }

    /// ⌘V with an image on the clipboard attaches it; with text, it pastes as always. (The text field alone
    /// only pastes text.)
    private func installPasteMonitor() {
        removePasteMonitor()
        pasteMonitor.value = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            guard event.modifierFlags.intersection(.deviceIndependentFlagsMask) == .command,
                  event.charactersIgnoringModifiers == "v",
                  focused else { return event }
            return chat.attachFromPasteboard() ? nil : event
        }
    }

    private func removePasteMonitor() {
        if let monitor = pasteMonitor.value { NSEvent.removeMonitor(monitor) }
        pasteMonitor.value = nil
    }

    private var queue: some View {
        ScrollView {
            VStack(spacing: 0) {
                ForEach(chat.queued, id: \.id) { item in
                    HStack(spacing: 6) {
                        Image(systemName: "text.line.first.and.arrowtriangle.forward")
                            .font(.system(size: 11)).foregroundStyle(.secondary)
                        if !item.attachments.isEmpty {
                            FileChip(path: item.attachments[0], compact: true)
                        }
                        Text(item.text).font(.system(size: 12)).lineLimit(1)
                            .frame(maxWidth: .infinity, alignment: .leading).help(item.text)
                        Button { chat.redirectQueued(item.id) } label: {
                            Label("Redirigir", systemImage: "arrow.turn.down.right").font(.system(size: 11))
                        }
                        .buttonStyle(.borderless).foregroundStyle(.secondary)
                        .tip("Detener la respuesta actual y enviar este mensaje")
                        Button { chat.removeQueued(item.id) } label: {
                            Image(systemName: "trash").frame(width: 22, height: 24)
                        }
                        .buttonStyle(.borderless).foregroundStyle(.secondary).tip("Eliminar de la cola")
                        Menu {
                            Button("Editar mensaje", systemImage: "pencil") { chat.editQueued(item.id); focused = true }
                            Button("Copiar mensaje", systemImage: "doc.on.doc") { chat.copyQueued(item.text) }
                        } label: {
                            Image(systemName: "ellipsis").frame(width: 22, height: 24)
                        }
                        .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
                        .foregroundStyle(.secondary).tip("Más opciones")
                    }
                    .padding(.horizontal, 8).frame(height: 40)
                }
            }
        }
        .frame(width: ChatMetrics.composerWidth - 24, height: min(CGFloat(chat.queued.count) * 40, 120))
        .padding(.bottom, 14)
        .background(Color.dynamic(light: "#F1F1F3", dark: "#303034"), in: RoundedRectangle(cornerRadius: 16))
        .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder(.primary.opacity(0.16), lineWidth: 1))
        .accessibilityLabel("Mensajes en cola")
    }

    private var field: some View {
        HStack(alignment: .bottom, spacing: 6) {
            Button(action: pickFiles) {
                Image(systemName: "plus")
                    .font(.system(size: 14, weight: .medium))
                    .frame(width: 28, height: 28)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.borderless)
            .foregroundStyle(.secondary)
            .tip("Adjuntar archivos (imágenes, PDF, texto)")
            TextField("Pregúntale a Buddy", text: $chat.draft, axis: .vertical)
                .textFieldStyle(.plain)
                .font(.body)
                .lineLimit(1...6)
                .focused($focused)
                .onSubmit { chat.send() }
                .padding(.vertical, 5)
            if chat.streaming {
                Button(action: chat.stop) {
                    Image(systemName: "stop.fill").font(.system(size: 10, weight: .bold)).frame(width: 24, height: 24)
                }
                .buttonStyle(.borderless)
                .foregroundStyle(.secondary)
                .tip("Detener y vaciar la cola")
            }
            Button(action: chat.send) {
                Image(systemName: chat.streaming ? "text.line.first.and.arrowtriangle.forward" : "arrow.up")
                    .font(.system(size: 12, weight: .bold))
                    .frame(width: 16, height: 16)
            }
            .disabled(empty)
            .tip(chat.streaming ? "Añadir a la cola (↩)" : "Enviar (↩)")
            .buttonStyle(.borderedProminent)
            .buttonBorderShape(.circle)
            .controlSize(.small)
        }
    }

    private func pickFiles() {
        let panel = NSOpenPanel()
        panel.title = "Adjuntar a Buddy"
        panel.prompt = "Adjuntar"
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = true
        guard panel.runModal() == .OK else { return }
        chat.attach(panel.urls)
        focused = true
    }
}

/// Holds the paste monitor between view updates.
final class MonitorBox {
    var value: Any?
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
            VStack(alignment: .trailing, spacing: 6) {
                if !message.files.isEmpty {
                    HStack(spacing: 6) {
                        Spacer(minLength: 48)
                        ForEach(message.files, id: \.self) { FileChip(path: $0, onRemove: nil) }
                    }
                }
            HStack {
                Spacer(minLength: 48)
                Text(message.content)
                    .font(.body)
                    .textSelection(.enabled)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .background(.quaternary, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            }
            }
        } else {
            VStack(alignment: .leading, spacing: 6) {
                AuthorLine(name: message.author ?? "Buddy")
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
                    MessageActions(text: message.content, provider: message.provider, canRegenerate: isLastAnswer && canRegenerate,
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

/// Buddy's face and the agent's name.
private struct AuthorLine: View {
    let name: String

    var body: some View {
        HStack(spacing: 6) {
            AvatarView(size: 18)
                .tip(name == "Buddy" ? "Buddy" : "\(name), del equipo de Buddy")
            Text(name)
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
        }
    }
}

/// Copy and write again, under an answer, followed by the mark of the service that wrote it.
private struct MessageActions: View {
    let text: String
    let provider: String?
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
            ProviderMark(provider: provider)
                .padding(.leading, 6)
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
            if !activity.symbol.isEmpty {
                Image(systemName: activity.symbol)
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(.tint)
                    .symbolEffect(.pulse, options: .repeating, isActive: !reduceMotion)
                    .frame(width: 16)
            }
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

/// A file as a small chip: its icon and name; with a remove button while it waits in the composer.
struct FileChip: View {
    let path: String
    var compact = false
    var onRemove: (() -> Void)? = nil

    /// Images show themselves instead of the file icon.
    private static func thumbnail(_ path: String) -> NSImage? {
        guard ["png", "jpg", "jpeg", "gif", "webp", "heic", "tiff"].contains((path as NSString).pathExtension.lowercased()) else { return nil }
        return NSImage(contentsOfFile: path)
    }

    var body: some View {
        HStack(spacing: 6) {
            if let thumb = Self.thumbnail(path) {
                Image(nsImage: thumb)
                    .resizable()
                    .scaledToFill()
                    .frame(width: 24, height: 24)
                    .clipShape(RoundedRectangle(cornerRadius: 4))
            } else {
                Image(nsImage: NSWorkspace.shared.icon(forFile: path))
                    .resizable()
                    .frame(width: 16, height: 16)
            }
            if !compact {
            Text((path as NSString).lastPathComponent)
                .font(.system(size: 12))
                .lineLimit(1)
                .truncationMode(.middle)
                .frame(maxWidth: 160, alignment: .leading)
            }
            if let onRemove {
                Button(action: onRemove) {
                    Image(systemName: "xmark").font(.system(size: 9, weight: .bold))
                }
                .buttonStyle(.borderless)
                .foregroundStyle(.secondary)
                .tip("Quitar")
            }
        }
        .padding(.horizontal, compact ? 4 : 8)
        .frame(height: compact ? 30 : 26)
        .background(.quaternary, in: Capsule())
        .tip(path)
        .onTapGesture(count: 2) { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)]) }
    }
}
