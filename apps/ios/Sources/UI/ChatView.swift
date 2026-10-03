import SwiftUI

/// A conversation with Buddy on the paired machine: its answer arrives as it is written there.
struct ChatView: View {
    @Environment(AppModel.self) private var model
    @State private var draft = ""
    @FocusState private var typing: Bool

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 14) {
                    if model.bubbles.isEmpty {
                        VStack(spacing: 12) {
                            PixelSpriteView(sprite: model.sprite).frame(width: 96, height: 96)
                            Text("¿En qué te ayudo?").font(.title3.weight(.semibold))
                            ForEach(model.suggestions.prefix(3), id: \.self) { suggestion in
                                Button(suggestion) { send(suggestion) }
                                    .buttonStyle(.bordered)
                                    .buttonBorderShape(.capsule)
                            }
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.top, 40)
                    }
                    ForEach(model.bubbles) { bubble in
                        BubbleView(bubble: bubble, sprite: model.sprite).id(bubble.id)
                    }
                    if let activity = model.activity {
                        HStack(spacing: 8) {
                            ProgressView().controlSize(.small)
                            Text(activity).font(.footnote).foregroundStyle(.secondary)
                        }
                        .id("activity")
                        .accessibilityElement(children: .combine)
                    }
                    if let error = model.chatError {
                        Label(error, systemImage: "exclamationmark.triangle").font(.footnote).foregroundStyle(.red)
                    }
                    Color.clear.frame(height: 1).id("end")
                }
                .padding()
            }
            .scrollDismissesKeyboard(.interactively)
            .onChange(of: model.bubbles.last?.text) { _, _ in proxy.scrollTo("end", anchor: .bottom) }
            .onChange(of: model.bubbles.count) { _, _ in withAnimation { proxy.scrollTo("end", anchor: .bottom) } }
        }
        .safeAreaInset(edge: .bottom) { composer }
        .navigationTitle("Buddy")
        .navigationBarTitleDisplayMode(.inline)
        // The conversation takes the whole screen: the tabs come back with the back button.
        .toolbar(.hidden, for: .tabBar)
    }

    private var composer: some View {
        HStack(alignment: .bottom, spacing: 8) {
            TextField("Mensaje para Buddy", text: $draft, axis: .vertical)
                .lineLimit(1...6)
                .focused($typing)
                .padding(.horizontal, 14).padding(.vertical, 10)
                .glassEffect(.regular, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
            if model.answering {
                Button { model.stop() } label: { Image(systemName: "stop.fill").frame(width: 22, height: 22) }
                    .buttonStyle(.glassProminent)
                    .buttonBorderShape(.circle)
                    .accessibilityLabel("Detener la respuesta")
            } else {
                Button { send(draft) } label: { Image(systemName: "arrow.up").fontWeight(.semibold).frame(width: 22, height: 22) }
                    .buttonStyle(.glassProminent)
                    .buttonBorderShape(.circle)
                    .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || model.status != .ready)
                    .accessibilityLabel("Enviar")
            }
        }
        .padding(.horizontal).padding(.vertical, 8)
    }

    private func send(_ text: String) {
        draft = ""
        Task { await model.send(text) }
    }
}

struct BubbleView: View {
    let bubble: Bubble
    let sprite: Sprite?

    var body: some View {
        if bubble.role == "user" {
            Text(bubble.text)
                .padding(.horizontal, 14).padding(.vertical, 10)
                .background(.tint, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                .foregroundStyle(.white)
                .frame(maxWidth: .infinity, alignment: .trailing)
                .padding(.leading, 48)
        } else {
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 6) {
                    PixelSpriteView(sprite: sprite).frame(width: 22, height: 22)
                    Text(bubble.agentName.isEmpty ? "Buddy" : bubble.agentName).font(.footnote.weight(.semibold))
                    if let took = bubble.took { Text("· \(took)").font(.footnote).foregroundStyle(.secondary) }
                }
                Text(Self.rich(bubble.text))
                    .foregroundStyle(bubble.failed ? .red : .primary)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .accessibilityElement(children: .combine)
        }
    }

    /// The answer with its bold, italics, code and links; anything Markdown cannot read stays as written.
    static func rich(_ text: String) -> AttributedString {
        let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace, failurePolicy: .returnPartiallyParsedIfPossible)
        return (try? AttributedString(markdown: text, options: options)) ?? AttributedString(text)
    }
}

/// The machine's chats, newest first; they read from its own history.
struct ChatsView: View {
    @Environment(AppModel.self) private var model
    @State private var search = ""
    @State private var opened: ChatSummary?
    @State private var composing = false

    var body: some View {
        List(shown) { chat in
            Button {
                opened = chat
            } label: {
                VStack(alignment: .leading, spacing: 4) {
                    Text(chat.title).font(.body.weight(.medium)).lineLimit(1)
                    Text(chat.preview).font(.subheadline).foregroundStyle(.secondary).lineLimit(2)
                }
            }
            .foregroundStyle(.primary)
        }
        .overlay {
            if model.chats.isEmpty {
                ContentUnavailableView("Sin chats", systemImage: "bubble.left.and.bubble.right",
                                       description: Text(model.status == .ready ? "Lo que hables con Buddy en \(model.current?.name ?? "tu equipo") aparecerá aquí." : "Conéctate a tu equipo para ver sus chats."))
            }
        }
        .searchable(text: $search, prompt: "Buscar en los chats")
        .navigationTitle("Chats")
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    model.newChat()
                    composing = true
                } label: {
                    Label("Chat nuevo", systemImage: "square.and.pencil")
                }
                .disabled(model.status != .ready)
            }
        }
        .refreshable { await model.refresh() }
        .navigationDestination(item: $opened) { chat in
            ChatView().task { await model.open(chat) }
        }
        .navigationDestination(isPresented: $composing) { ChatView() }
    }

    private var shown: [ChatSummary] {
        let words = search.trimmingCharacters(in: .whitespaces).lowercased()
        return words.isEmpty ? model.chats : model.chats.filter { $0.title.lowercased().contains(words) || $0.preview.lowercased().contains(words) }
    }
}

extension ChatSummary: Hashable {
    func hash(into hasher: inout Hasher) { hasher.combine(id) }
}
