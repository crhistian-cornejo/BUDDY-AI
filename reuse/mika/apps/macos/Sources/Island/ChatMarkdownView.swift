import SwiftUI
import AppKit

// How an assistant answer is drawn: markdown (bold, italic, lists, tables, code, quotes), LaTeX equations, a status line
// while the agent works, and the pages it used as a stack of site icons. Everything is built from the trees the parsers in
// Sources/Providers/Markdown produce, with no library. Text is always text: no HTML, and links are never clickable
// inside the answer (the sources row is the only way out, and it only opens http/https).

private enum Ink {
    static let body = Color(hex: "#B0B5BE")
    static let strong = Color(hex: "#F1F2F4")
    static let faint = Color(hex: "#6B7079")
    static let code = Color(hex: "#D5D9E0")
    static let math = Color(hex: "#E3E6EB")
    static let link = Color(hex: "#8AB4F8")
    static let ok = Color(hex: "#7FD18B")
}

/// The colours of a code token. The same palette as the Windows app (answer.css, `.tk-*`).
private enum CodeInk {
    static func color(_ kind: CodeTokenKind) -> Color {
        switch kind {
        case .kw: return Color(hex: "#C586C0")
        case .str: return Color(hex: "#CE9178")
        case .com: return Color(hex: "#6A9955")
        case .num: return Color(hex: "#B5CEA8")
        case .type: return Color(hex: "#4EC9B0")
        case .fn: return Color(hex: "#DCDCAA")
        case .prop: return Color(hex: "#9CDCFE")
        case .tag: return Color(hex: "#569CD6")
        case .add: return Color(hex: "#7FD18B")
        case .del: return Color(hex: "#F28B82")
        case .hunk: return Color(hex: "#8AB4F8")
        case .plain: return Ink.code
        }
    }

    static func attributed(_ code: String, language: String?) -> AttributedString {
        var out = AttributedString()
        for token in CodeHighlighter.highlight(code, language: language) {
            var piece = AttributedString(token.text)
            piece.foregroundColor = color(token.kind)
            if token.kind == .com { piece.font = .system(size: 11, design: .monospaced).italic() }
            if token.kind == .add { piece.backgroundColor = Color(hex: "#7FD18B").opacity(0.1) }
            if token.kind == .del { piece.backgroundColor = Color(hex: "#F28B82").opacity(0.1) }
            out.append(piece)
        }
        return out
    }
}

/// Selecting text costs layout work on every frame, so it is off while the answer is still being written.
private extension View {
    @ViewBuilder func selectable(_ on: Bool) -> some View {
        if on { textSelection(.enabled) } else { textSelection(.disabled) }
    }
}

// MARK: - the answer

/// One assistant message: the status line, the markdown and the sources. Equatable so that finished answers are not
/// drawn again while a new one streams in.
struct AssistantBubble: View, Equatable {
    let message: ChatMessage

    var body: some View {
        let cleaned = MarkdownSources.clean(message.content)
        let sources = message.sources + cleaned.sources.filter { s in !message.sources.contains { $0.url == s.url } }
        VStack(alignment: .leading, spacing: 7) {
            if let status = message.status { AnswerStatusView(text: status) }
            if !cleaned.text.isEmpty { MarkdownView(text: cleaned.text, streaming: message.isStreaming) }
            SourcesRow(sources: sources)
        }
        .animation(.easeOut(duration: 0.2), value: sources.count)
    }
}

/// "Buscando: alianza lima hoy…" (or "Pensando…") the way the Claude and ChatGPT apps show what the model is doing: grey
/// text with a light band sweeping across it. With reduced motion, plain text. Twin of `.work-status` on Windows.
struct AnswerStatusView: View {
    let text: String
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private static var base: Color { Color(hex: "#6B7079") }
    private static var light: Color { Color(hex: "#EEF0F3") }
    private static let period = 1.9

    var body: some View {
        let label = Text(text)
            .font(.system(size: 12, weight: .medium))
            .lineLimit(1)
            .truncationMode(.tail)
        if reduceMotion {
            label.foregroundColor(Color(hex: "#8E939C"))
        } else {
            label
                .foregroundColor(Self.base)
                .overlay {
                    // A band 2.5 times as wide as the text, its light stripe at the middle, slides from -1.5 to +2.25
                    // widths (CSS background-position 100% → -150%), so the light crosses the text and then rests.
                    GeometryReader { geo in
                        TimelineView(.animation) { timeline in
                            let t = timeline.date.timeIntervalSinceReferenceDate / Self.period
                            let w = geo.size.width
                            LinearGradient(stops: [.init(color: Self.base, location: 0), .init(color: Self.base, location: 0.35),
                                                   .init(color: Self.light, location: 0.5), .init(color: Self.base, location: 0.65),
                                                   .init(color: Self.base, location: 1)],
                                           startPoint: .leading, endPoint: .trailing)
                                .frame(width: w * 2.5, height: geo.size.height)
                                .offset(x: -1.5 * w + 3.75 * w * (t - t.rounded(.down)))
                        }
                    }
                    .mask(label)
                    .allowsHitTesting(false)
                }
        }
    }
}

// MARK: - sources

/// The pages an answer used, as a stack of round icons: the newest (the last page fetched or searched) on top with its
/// site's name beside it, older ones under it and, past eight, as "+N". The stack spreads out on hover so every site can
/// be told apart and clicked. Twin of `sourcesRow` in apps/windows/src/views/answer.ts. The icon is the site's own
/// logo: the well-known sites have a mark drawn in code (SourceLogos: instant, no request); any other site shows a letter
/// tile and, with "Íconos de fuentes" on, its own favicon (SourceIcons, fetched from the site itself) once it arrives.
/// A click opens the page in the browser (http/https only).
struct SourcesRow: View {
    let sources: [ChatSource]
    private let limit = 8
    @State private var spread = false

    var body: some View {
        if !sources.isEmpty {
            let shown = Array(sources.suffix(limit))
            let hidden = sources.count - shown.count
            HStack(spacing: 7) {
                if hidden > 0 {
                    Text(verbatim: "+\(hidden)")
                        .font(.system(size: 10.5, weight: .medium))
                        .foregroundColor(Ink.faint)
                        .tip(sources.prefix(hidden).map(\.host).joined(separator: "\n"))
                }
                HStack(spacing: spread ? 3 : -9) {
                    ForEach(Array(shown.enumerated()), id: \.element.url) { index, source in
                        SourceChip(source: source)
                            .zIndex(Double(index))          // the newest on top
                            .transition(.scale(scale: 0.6).combined(with: .opacity))
                    }
                }
                .onHover { inside in withAnimation(.spring(response: 0.22, dampingFraction: 0.8)) { spread = inside } }
                if let last = shown.last {
                    Text(verbatim: last.host)
                        .font(.system(size: 10.5))
                        .foregroundColor(Color(hex: spread ? "#C4C8D0" : "#8E939C"))
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .frame(maxWidth: 160, alignment: .leading)
                }
            }
            .frame(minHeight: 22)
        }
    }
}

/// Site icons already asked for, by host: one request per site, whatever the number of answers that cite it.
actor SourceIconLoader {
    static let shared = SourceIconLoader()
    private var asked: [String: Task<Data?, Never>] = [:]

    func icon(for source: ChatSource) async -> Data? {
        guard SourceIcons.enabled else { return nil }
        if let task = asked[source.host] { return await task.value }
        let url = source.url
        let task = Task.detached(priority: .utility) { await SourceIcons.icon(for: url) }
        asked[source.host] = task
        return await task.value
    }
}

struct SourceChip: View {
    let source: ChatSource
    @AppStorage(SourceIcons.settingKey) private var iconsOn = true
    @State private var hovered = false
    @State private var icon: NSImage?

    /// The mark drawn in code for this site, if it has one.
    private var logo: SiteLogo? { SourceLogos.logo(for: source.host) }

    /// The letter tile's colour: a hash of the site's registrable domain (the same for every subdomain).
    private var tint: Color { Color(hex: SourceLogos.fallbackColor(for: source.host)) }

    private var initial: String { SourceLogos.fallbackInitial(for: source.host) }

    var body: some View {
        Button(action: open) {
            ZStack {
                if let logo {
                    SiteLogoView(logo: logo)
                        .frame(width: 20, height: 20)
                        .clipShape(Circle())
                } else if iconsOn, let icon {
                    Circle().fill(Color(hex: "#F5F6F8"))
                    Image(nsImage: icon)
                        .resizable()
                        .interpolation(.high)
                        .scaledToFit()
                        .frame(width: 14, height: 14)
                } else {
                    Circle().fill(tint)
                    Text(verbatim: initial)
                        .font(.system(size: 10, weight: .bold))
                        .foregroundColor(.white)
                }
            }
            .frame(width: 20, height: 20)
            .overlay(Circle().stroke(Color.black.opacity(0.55), lineWidth: 1.5))   // keeps overlapping chips apart
            .scaleEffect(hovered ? 1.08 : 1)
            .offset(y: hovered ? -1 : 0)
        }
        .buttonStyle(.plain)
        .onHover { inside in withAnimation(.easeOut(duration: 0.12)) { hovered = inside } }
        .tip("\(source.title.isEmpty ? source.host : source.title)\n\(source.host)")
        .contextMenu {
            Button("Abrir en el navegador", action: open)
            Button("Copiar enlace") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(source.url, forType: .string)
            }
        }
        .task(id: iconsOn) {
            // A site with a drawn logo never asks for its favicon.
            guard logo == nil, iconsOn, icon == nil, let data = await SourceIconLoader.shared.icon(for: source) else { return }
            icon = NSImage(data: data)
        }
    }

    private func open() {
        guard let url = URL(string: source.url), ["http", "https"].contains(url.scheme?.lowercased() ?? "") else { return }
        NSWorkspace.shared.open(url)
    }
}

/// A logo drawn in code (`SourceLogos`): its background and shapes in a 24x24 box scaled to the view. The caller clips it to
/// the circle. Twin of `logoSvg` in apps/windows/src/views/answer.ts.
struct SiteLogoView: View {
    let logo: SiteLogo

    var body: some View {
        Canvas { context, size in
            let scale = min(size.width, size.height) / 24
            context.scaleBy(x: scale, y: scale)
            context.fill(Path(CGRect(x: 0, y: 0, width: 24, height: 24)), with: .color(Color(hex: logo.bg)))
            for shape in logo.shapes {
                guard let segments = SVGPath.parse(shape.d) else { continue }
                let path = Path { p in
                    for segment in segments {
                        switch segment {
                        case .move(let to): p.move(to: to)
                        case .line(let to): p.addLine(to: to)
                        case .cubic(let c1, let c2, let to): p.addCurve(to: to, control1: c1, control2: c2)
                        case .quad(let c, let to): p.addQuadCurve(to: to, control: c)
                        case .close: p.closeSubpath()
                        }
                    }
                }
                if let stroke = shape.stroke {
                    context.stroke(path, with: .color(Color(hex: stroke)),
                                   style: StrokeStyle(lineWidth: shape.width, lineCap: .round, lineJoin: .round))
                } else {
                    context.fill(path, with: .color(Color(hex: shape.fill ?? "#ffffff")))
                }
            }
        }
        .allowsHitTesting(false)
    }
}

// MARK: - markdown

struct MarkdownView: View {
    let text: String
    var streaming = false

    var body: some View {
        let blocks = MarkdownParser.parse(text)
        VStack(alignment: .leading, spacing: 7) {
            ForEach(Array(blocks.enumerated()), id: \.offset) { index, block in
                // Only the block that is being written changes: the rest are not laid out again while text streams in.
                MarkdownBlockView(block: block, live: streaming && index == blocks.count - 1).equatable()
            }
        }
    }
}

private struct MarkdownBlocks: View {
    let blocks: [MarkdownBlock]
    var depth = 0
    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            ForEach(Array(blocks.enumerated()), id: \.offset) { _, block in MarkdownBlockView(block: block, depth: depth).equatable() }
        }
    }
}

struct MarkdownBlockView: View, Equatable {
    let block: MarkdownBlock
    var live = false
    /// How deep the list this block belongs to is nested: it picks the bullet.
    var depth = 0
    private let size: CGFloat = 12.5

    var body: some View {
        switch block {
        case .heading(let level, let text):
            InlineText(source: text, size: level == 1 ? 15 : level == 2 ? 14 : level == 3 ? 13 : 12.5, weight: .semibold,
                       color: level >= 4 ? Ink.body : Ink.strong, live: live)
                .padding(.top, 4)
        case .paragraph(let text):
            InlineText(source: text, size: size, color: Ink.body, live: live)
        case .bulletList(let items):
            listView(items) { _ in ["•", "◦", "▪"][min(depth, 2)] }
        case .orderedList(let start, let items):
            listView(items) { "\(start + $0)." }
        case .codeBlock(let language, let code):
            codeView(language: language, code: code)
        case .quote(let blocks):
            HStack(alignment: .top, spacing: 8) {
                RoundedRectangle(cornerRadius: 1).fill(Color.white.opacity(0.22)).frame(width: 2)
                MarkdownBlocks(blocks: blocks).opacity(0.85)
            }
            .fixedSize(horizontal: false, vertical: true)
        case .rule:
            Rectangle().fill(Color.white.opacity(0.12)).frame(height: 1).padding(.vertical, 3)
        case .table(let header, let alignments, let rows):
            tableView(header: header, alignments: alignments, rows: rows)
        case .math(let latex):
            MathBlockView(latex: latex)
        }
    }

    private func listView(_ items: [MarkdownListItem], marker: @escaping (Int) -> String) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            ForEach(Array(items.enumerated()), id: \.offset) { index, item in
                let task = MarkdownTask.marker(item.text)
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    if let task {
                        Image(systemName: task.checked ? "checkmark.square.fill" : "square")
                            .font(.system(size: size - 1))
                            .foregroundColor(task.checked ? Ink.ok : Ink.faint)
                            .frame(minWidth: 13, alignment: .trailing)
                            .accessibilityLabel(task.checked ? "Hecho" : "Pendiente")
                    } else {
                        Text(marker(index))
                            .font(.system(size: size).monospacedDigit())
                            .foregroundColor(Ink.faint)
                            .frame(minWidth: 13, alignment: .trailing)
                    }
                    VStack(alignment: .leading, spacing: 4) {
                        InlineText(source: task?.rest ?? item.text, size: size, color: task?.checked == true ? Ink.faint : Ink.body,
                                   live: live && index == items.count - 1)
                        if !item.children.isEmpty { MarkdownBlocks(blocks: item.children, depth: depth + 1) }
                    }
                }
            }
        }
    }

    private func codeView(language: String?, code: String) -> some View {
        CodeBlockView(language: language, code: code, live: live)
    }

    private func tableView(header: [String], alignments: [TableAlignment], rows: [[String]]) -> some View {
        func alignment(_ column: Int) -> HorizontalAlignment {
            switch column < alignments.count ? alignments[column] : .leading {
            case .leading: return .leading
            case .center: return .center
            case .trailing: return .trailing
            }
        }
        // Every cell fills its column and paints its own band, so the header and the zebra rows are solid bands, not blocks
        // as wide as their text.
        func cell(_ source: String, column: Int, header: Bool, shade: Double) -> some View {
            InlineText(source: source, size: 11.5, weight: header ? .semibold : .regular, color: header ? Ink.strong : Ink.body)
                .padding(.horizontal, 12).padding(.vertical, header ? 5 : 4)
                .frame(maxWidth: .infinity, alignment: Alignment(horizontal: alignment(column), vertical: .center))
                .background(Color.white.opacity(shade))
                .gridColumnAlignment(alignment(column))
        }
        return ScrollView(.horizontal, showsIndicators: false) {
            Grid(horizontalSpacing: 0, verticalSpacing: 0) {
                GridRow {
                    ForEach(Array(header.enumerated()), id: \.offset) { column, text in cell(text, column: column, header: true, shade: 0.08) }
                }
                Rectangle().fill(Color.white.opacity(0.15)).frame(height: 1).gridCellColumns(max(header.count, 1))
                ForEach(Array(rows.enumerated()), id: \.offset) { rowIndex, row in
                    GridRow {
                        ForEach(Array(row.enumerated()), id: \.offset) { column, text in
                            cell(text, column: column, header: false, shade: rowIndex % 2 == 1 ? 0.04 : 0)
                        }
                    }
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color.white.opacity(0.12), lineWidth: 1))
        }
    }
}

/// One paragraph of inline markdown: bold, italic, `code`, ~~strike~~ and inline equations. Never clickable.
struct InlineText: View {
    let source: String
    var size: CGFloat = 12.5
    var weight: Font.Weight = .regular
    var color: Color = Ink.body
    var live = false

    /// The line; with a small service icon in front of each workspace link (Docs, Drive, Calendar, Meet…).
    private var content: Text {
        let chunks = InlineMarkdown.chunks(source, size: size, color: color, weight: weight)
        guard chunks.contains(where: { if case .mark = $0 { return true }; return false }) else {
            return Text(InlineMarkdown.attributed(source, size: size, color: color, weight: weight))
        }
        return chunks.reduce(Text("")) { text, chunk in
            switch chunk {
            case .text(let attributed): return text + Text(attributed)
            case .mark(let mark): return text + Text(Image(nsImage: BrandIcons.image(mark, size: size * 1.05))).baselineOffset(-size * 0.14) + Text(" ")
            }
        }
    }

    var body: some View {
        content
            .environment(\.openURL, OpenURLAction { url in
                guard LinkPolicy.keeps(url.absoluteString) else { return .discarded }
                Task { @MainActor in ExternalLink.open(url) }
                return .handled
            })
            .lineSpacing(2)
            .fixedSize(horizontal: false, vertical: true)
            .selectable(!live)
    }
}

/// A run of an inline line: styled text, or the icon of the service a workspace link points to (placed before the link).
enum InlineChunk {
    case text(AttributedString)
    case mark(BrandMark)
}

enum InlineMarkdown {
    /// The line as chunks: the same text as `attributed`, with a service icon in front of each workspace link.
    static func chunks(_ source: String, size: CGFloat, color: Color, weight: Font.Weight = .regular) -> [InlineChunk] {
        var out: [InlineChunk] = []
        var pending = AttributedString()
        func flush() { if !pending.characters.isEmpty { out.append(.text(pending)); pending = AttributedString() } }
        for part in MarkdownInline.segments(source) {
            switch part {
            case .math(let latex): pending.append(mathRun(latex, size: size))
            case .text(let text):
                let styledText = styled(text, size: size, color: color, weight: weight)
                var last: URL?
                for run in styledText.runs {
                    if let link = run.link, link != last, let service = LinkPolicy.service(for: link.absoluteString),
                       let mark = BrandMark(rawValue: service.rawValue) {
                        flush()
                        out.append(.mark(mark))
                    }
                    last = run.link
                    pending.append(AttributedString(styledText[run.range]))
                }
            }
        }
        flush()
        return out
    }

    static func attributed(_ source: String, size: CGFloat, color: Color, weight: Font.Weight = .regular) -> AttributedString {
        var result = AttributedString()
        for part in MarkdownInline.segments(source) {
            switch part {
            case .text(let text): result.append(styled(text, size: size, color: color, weight: weight))
            case .math(let latex): result.append(mathRun(latex, size: size))
            }
        }
        return result
    }

    private static func styled(_ text: String, size: CGFloat, color: Color, weight: Font.Weight) -> AttributedString {
        let options = AttributedString.MarkdownParsingOptions(allowsExtendedAttributes: false,
                                                              interpretedSyntax: .inlineOnlyPreservingWhitespace,
                                                              failurePolicy: .returnPartiallyParsedIfPossible)
        var out = (try? AttributedString(markdown: text, options: options)) ?? AttributedString(text)
        out.foregroundColor = color
        out.font = .system(size: size, weight: weight)
        var hosts: [(Range<AttributedString.Index>, String)] = []
        for run in out.runs {
            let intent = run.inlinePresentationIntent ?? []
            let range = run.range
            if let link = run.link {
                // No address is clickable in an answer, except the user's own workspaces (LinkPolicy); the rest show their
                // label in the link colour with the site beside it.
                let kept = LinkPolicy.keeps(link.absoluteString)
                if !kept { out[range].link = nil }
                out[range].foregroundColor = Ink.link
                out[range].underlineStyle = .single
                if !kept, let host = linkHost(link) { hosts.append((range, host)) }
            }
            if run.imageURL != nil {
                out[range].imageURL = nil
                out[range].foregroundColor = Ink.faint
                out[range].font = .system(size: size).italic()
            }
            if intent.contains(.code) {
                out[range].font = .system(size: size * 0.92, design: .monospaced)
                out[range].foregroundColor = Ink.code
                out[range].backgroundColor = Color.white.opacity(0.1)
                continue
            }
            let bold = intent.contains(.stronglyEmphasized), italic = intent.contains(.emphasized)
            if bold || italic {
                var font = Font.system(size: size, weight: bold ? .bold : weight)
                if italic { font = font.italic() }
                out[range].font = font
            }
            if bold { out[range].foregroundColor = Ink.strong }
            if intent.contains(.strikethrough) { out[range].strikethroughStyle = .single }
        }
        for (range, host) in hosts.reversed() {
            var note = AttributedString(" " + host)
            note.font = .system(size: size * 0.85)
            note.foregroundColor = Ink.faint
            out.insert(note, at: range.upperBound)
        }
        return out
    }

    /// The site a link points to, for the small note beside its label; nil for anything but http(s).
    static func linkHost(_ url: URL) -> String? {
        guard let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https", let host = url.host?.lowercased() else { return nil }
        return host.hasPrefix("www.") ? String(host.dropFirst(4)) : host
    }

    /// An equation inside a line: its Unicode form in a serif face (x², a⁄b, √x). Unknown LaTeX shows its source.
    private static func mathRun(_ latex: String, size: CGFloat) -> AttributedString {
        if let node = MathParser.parse(latex) {
            var run = AttributedString(node.plainText)
            run.font = .system(size: size * 1.05, design: .serif)
            run.foregroundColor = Ink.math
            return run
        }
        var run = AttributedString(latex)
        run.font = .system(size: size * 0.92, design: .monospaced)
        run.foregroundColor = Ink.code
        return run
    }
}

// MARK: - equations

private struct MathBlockView: View {
    let latex: String

    var body: some View {
        if let node = MathParser.parse(latex) {
            ViewThatFits(in: .horizontal) {
                MathView(node: node, size: 15)
                ScrollView(.horizontal, showsIndicators: false) { MathView(node: node, size: 15) }
            }
            .frame(maxWidth: .infinity, alignment: .center)
            .padding(.vertical, 4)
            .contextMenu {
                Button("Copiar LaTeX") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(latex, forType: .string)
                }
            }
        } else {
            Text(latex)
                .font(.system(size: 11, design: .monospaced))
                .foregroundColor(Ink.code)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

struct MathView: View {
    let node: MathNode
    var size: CGFloat = 14

    var body: some View {
        MathNodeView(node: node, ctx: MathContext(size: size))
            .foregroundColor(Ink.math)
            .fixedSize()
    }
}

struct MathContext {
    var size: CGFloat
    var bold = false
    var roman = false
    var compact = false
    var script: MathContext { MathContext(size: size * 0.72, bold: bold, roman: roman, compact: true) }
    var part: MathContext { MathContext(size: size * (compact ? 0.9 : 0.92), bold: bold, roman: roman, compact: compact) }
    /// How far above the baseline the middle of an operator or a fraction bar sits.
    var axis: CGFloat { size * 0.26 }
}

private struct MathNodeView: View {
    let node: MathNode
    let ctx: MathContext

    private func font(italic: Bool) -> Font {
        let base = Font.system(size: ctx.size, weight: ctx.bold ? .bold : .regular, design: .serif)
        return italic && !ctx.roman ? base.italic() : base
    }

    var body: some View {
        switch node {
        case .row(let items):
            HStack(alignment: .firstTextBaseline, spacing: 0) {
                ForEach(Array(items.enumerated()), id: \.offset) { _, item in MathNodeView(node: item, ctx: ctx) }
            }
        case .symbol(let glyph, let italic):
            Text(glyph).font(font(italic: italic)).fixedSize()
                .padding(.horizontal, !ctx.compact && MathParser.spacedOperators.contains(glyph) ? ctx.size * 0.2 : 0)
        case .text(let text):
            Text(text).font(.system(size: ctx.size * 0.95, design: .serif)).fixedSize()
        case .script(let base, let sup, let sub):
            ScriptLayout(size: ctx.size) {
                MathNodeView(node: base, ctx: ctx)
                if let sup { MathNodeView(node: sup, ctx: ctx.script) } else { Color.clear.frame(width: 0, height: 0) }
                if let sub { MathNodeView(node: sub, ctx: ctx.script) } else { Color.clear.frame(width: 0, height: 0) }
            }
        case .fraction(let top, let bottom):
            FractionLayout(size: ctx.size) {
                MathNodeView(node: top, ctx: ctx.part)
                MathNodeView(node: bottom, ctx: ctx.part)
                Rectangle().fill(.foreground).frame(height: 1)
            }
        case .sqrt(let inner, let index):
            RootLayout(size: ctx.size) {
                MathNodeView(node: inner, ctx: ctx)
                RadicalShape().stroke(style: StrokeStyle(lineWidth: max(1, ctx.size / 14), lineCap: .round, lineJoin: .round))
                if let index { MathNodeView(node: index, ctx: ctx.script) } else { Color.clear.frame(width: 0, height: 0) }
            }
        case .bigOp(let glyph, let sub, let sup):
            bigOperator(glyph: glyph, sub: sub, sup: sup)
        case .delimited(let left, let right, let inner):
            delimited(left: left, right: right) { MathNodeView(node: inner, ctx: ctx) }
        case .matrix(let rows, let left, let right, let alignment):
            delimited(left: left, right: right) { matrix(rows: rows, left: left, alignment: alignment) }
        case .accent(let accent, let inner):
            accented(accent, inner)
        case .styled(let style, let inner):
            MathNodeView(node: inner, ctx: MathContext(size: ctx.size, bold: ctx.bold || style == .bold,
                                                       roman: ctx.roman || style == .roman, compact: ctx.compact))
        }
    }

    private static let stacked: Set<String> = ["∑", "∏", "∐", "⋃", "⋂", "lim", "lim sup", "lim inf", "max", "min", "sup", "inf"]

    @ViewBuilder
    private func bigOperator(glyph: String, sub: MathNode?, sup: MathNode?) -> some View {
        let isWord = glyph.count > 1 || glyph.first?.isLetter == true
        let symbolSize = isWord ? ctx.size : ctx.size * 1.45
        let op = Text(glyph)
            .font(.system(size: symbolSize, design: .serif))
            .fixedSize()
            .alignmentGuide(.firstTextBaseline) { d in isWord ? d[.firstTextBaseline] : d.height / 2 + ctx.axis }
        if sub == nil && sup == nil {
            op
        } else if Self.stacked.contains(glyph) {
            LimitsLayout(axis: ctx.axis) {
                if let sup { MathNodeView(node: sup, ctx: ctx.script) } else { Color.clear.frame(width: 0, height: 0) }
                op
                if let sub { MathNodeView(node: sub, ctx: ctx.script) } else { Color.clear.frame(width: 0, height: 0) }
            }
        } else {
            ScriptLayout(size: ctx.size) {
                op
                if let sup { MathNodeView(node: sup, ctx: ctx.script) } else { Color.clear.frame(width: 0, height: 0) }
                if let sub { MathNodeView(node: sub, ctx: ctx.script) } else { Color.clear.frame(width: 0, height: 0) }
            }
        }
    }

    private func delimited<Content: View>(left: String, right: String, @ViewBuilder content: () -> Content) -> some View {
        let width = ctx.size * 0.34
        return content()
            .padding(.leading, left.isEmpty ? 0 : width + 1)
            .padding(.trailing, right.isEmpty ? 0 : width + 1)
            .padding(.vertical, 1)
            .overlay(alignment: .leading) { if !left.isEmpty { DelimiterShape(glyph: left, isLeft: true).stroke(style: StrokeStyle(lineWidth: max(1, ctx.size / 14), lineCap: .round)).frame(width: width) } }
            .overlay(alignment: .trailing) { if !right.isEmpty { DelimiterShape(glyph: right, isLeft: false).stroke(style: StrokeStyle(lineWidth: max(1, ctx.size / 14), lineCap: .round)).frame(width: width) } }
    }

    private func matrix(rows: [[MathNode]], left: String, alignment: MatrixAlignment) -> some View {
        let columns = rows.map(\.count).max() ?? 1
        func column(_ index: Int) -> HorizontalAlignment {
            switch alignment {
            case .center: return .center
            case .aligned: return left.isEmpty && columns > 1 && index == 0 ? .trailing : .leading
            }
        }
        return Grid(horizontalSpacing: ctx.size * 0.9, verticalSpacing: ctx.size * 0.3) {
            ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                GridRow {
                    ForEach(Array(row.enumerated()), id: \.offset) { index, cell in
                        MathNodeView(node: cell, ctx: ctx.part)
                            .gridColumnAlignment(column(index))
                    }
                }
            }
        }
        .alignmentGuide(.firstTextBaseline) { d in d.height / 2 + ctx.axis }
    }

    private func accented(_ accent: MathAccent, _ inner: MathNode) -> some View {
        let mark: String? = { switch accent {
            case .hat: return "^"
            case .tilde: return "~"
            case .dot: return "˙"
            case .ddot: return "¨"
            case .vec: return "→"
            case .bar, .overline, .underline: return nil
        } }()
        let lineWidth = max(1, ctx.size / 16)
        return MathNodeView(node: inner, ctx: ctx)
            .padding(.top, ctx.size * 0.3)
            .overlay(alignment: .top) {
                if let mark {
                    Text(mark).font(.system(size: ctx.size * (accent == .vec ? 0.6 : 0.85), design: .serif)).fixedSize()
                        .offset(y: accent == .hat || accent == .tilde ? ctx.size * 0.1 : 0)
                } else if accent != .underline {
                    Rectangle().fill(.foreground).frame(height: lineWidth).offset(y: ctx.size * 0.12)
                }
            }
            .overlay(alignment: .bottom) {
                if accent == .underline { Rectangle().fill(.foreground).frame(height: lineWidth) }
            }
    }
}

// MARK: - equation layouts (each reports its baseline so a row of them lines up)

/// A base with an exponent above and an index below, both to its right.
private struct ScriptLayout: Layout {
    var size: CGFloat

    private struct Geometry { var baseline: CGFloat; var supY: CGFloat; var subY: CGFloat; var baseY: CGFloat; var baseWidth: CGFloat; var size: CGSize }

    private func geometry(_ subviews: Subviews) -> Geometry {
        let base = subviews[0].sizeThatFits(.unspecified), sup = subviews[1].sizeThatFits(.unspecified), sub = subviews[2].sizeThatFits(.unspecified)
        let bb = subviews[0].dimensions(in: .unspecified)[.firstTextBaseline]
        let supB = subviews[1].dimensions(in: .unspecified)[.firstTextBaseline], subB = subviews[2].dimensions(in: .unspecified)[.firstTextBaseline]
        var minY: CGFloat = 0, maxY = base.height, supTop: CGFloat = 0, subTop: CGFloat = 0
        if sup.height > 0 {
            supTop = bb - max(size * 0.38, bb - supB) - supB
            minY = min(minY, supTop)
        }
        if sub.height > 0 {
            subTop = bb + size * 0.2 - subB
            if sup.height > 0 { subTop = max(subTop, supTop + sup.height + 1) }
            maxY = max(maxY, subTop + sub.height)
        }
        let width = base.width + max(sup.width, sub.width) + (sup.width > 0 || sub.width > 0 ? 1 : 0)
        return Geometry(baseline: bb - minY, supY: supTop - minY, subY: subTop - minY, baseY: -minY, baseWidth: base.width,
                        size: CGSize(width: width, height: maxY - minY))
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize { geometry(subviews).size }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let g = geometry(subviews)
        subviews[0].place(at: CGPoint(x: bounds.minX, y: bounds.minY + g.baseY), anchor: .topLeading, proposal: .unspecified)
        subviews[1].place(at: CGPoint(x: bounds.minX + g.baseWidth + 1, y: bounds.minY + g.supY), anchor: .topLeading, proposal: .unspecified)
        subviews[2].place(at: CGPoint(x: bounds.minX + g.baseWidth + 1, y: bounds.minY + g.subY), anchor: .topLeading, proposal: .unspecified)
    }

    func explicitAlignment(of guide: VerticalAlignment, in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGFloat? {
        guide == .firstTextBaseline || guide == .lastTextBaseline ? bounds.minY + geometry(subviews).baseline : nil
    }
}

/// numerator / bar / denominator; the bar sits on the math axis of the line it belongs to.
private struct FractionLayout: Layout {
    var size: CGFloat

    private func metrics(_ subviews: Subviews) -> (width: CGFloat, gap: CGFloat, top: CGSize, bottom: CGSize, barY: CGFloat, height: CGFloat) {
        let top = subviews[0].sizeThatFits(.unspecified), bottom = subviews[1].sizeThatFits(.unspecified)
        let gap = size * 0.14
        let width = max(top.width, bottom.width) + size * 0.3
        return (width, gap, top, bottom, top.height + gap, top.height + gap + 1 + gap + bottom.height)
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let m = metrics(subviews); return CGSize(width: m.width, height: m.height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let m = metrics(subviews)
        subviews[0].place(at: CGPoint(x: bounds.midX, y: bounds.minY), anchor: .top, proposal: .unspecified)
        subviews[1].place(at: CGPoint(x: bounds.midX, y: bounds.minY + m.barY + 1 + m.gap), anchor: .top, proposal: .unspecified)
        subviews[2].place(at: CGPoint(x: bounds.minX, y: bounds.minY + m.barY), anchor: .topLeading, proposal: ProposedViewSize(width: m.width, height: 1))
    }

    func explicitAlignment(of guide: VerticalAlignment, in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGFloat? {
        guard guide == .firstTextBaseline || guide == .lastTextBaseline else { return nil }
        return bounds.minY + metrics(subviews).barY + 0.5 + size * 0.26
    }
}

/// The radical sign, the bar over the radicand and an optional index.
private struct RootLayout: Layout {
    var size: CGFloat

    private func metrics(_ subviews: Subviews) -> (radicand: CGSize, baseline: CGFloat, sign: CGFloat, indexSize: CGSize, signX: CGFloat, pad: CGFloat, size: CGSize) {
        let radicand = subviews[0].sizeThatFits(.unspecified), index = subviews[2].sizeThatFits(.unspecified)
        let pad = size * 0.14, sign = size * 0.6
        let signX = max(0, index.width - sign * 0.5)
        let baseline = pad + subviews[0].dimensions(in: .unspecified)[.firstTextBaseline]
        return (radicand, baseline, sign, index, signX, pad, CGSize(width: signX + sign + radicand.width + 2, height: radicand.height + pad + 1))
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize { metrics(subviews).size }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let m = metrics(subviews)
        subviews[0].place(at: CGPoint(x: bounds.minX + m.signX + m.sign, y: bounds.minY + m.pad), anchor: .topLeading, proposal: .unspecified)
        subviews[1].place(at: CGPoint(x: bounds.minX + m.signX, y: bounds.minY), anchor: .topLeading,
                          proposal: ProposedViewSize(width: m.size.width - m.signX, height: m.size.height))
        subviews[2].place(at: CGPoint(x: bounds.minX, y: bounds.minY + m.size.height * 0.12), anchor: .topLeading, proposal: .unspecified)
    }

    func explicitAlignment(of guide: VerticalAlignment, in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGFloat? {
        guide == .firstTextBaseline || guide == .lastTextBaseline ? bounds.minY + metrics(subviews).baseline : nil
    }
}

/// Limits above and below an operator (∑, ∏, lim), centred on it.
private struct LimitsLayout: Layout {
    var axis: CGFloat

    private func metrics(_ subviews: Subviews) -> (width: CGFloat, gap: CGFloat, sup: CGSize, op: CGSize, sub: CGSize, baseline: CGFloat, height: CGFloat) {
        let sup = subviews[0].sizeThatFits(.unspecified), op = subviews[1].sizeThatFits(.unspecified), sub = subviews[2].sizeThatFits(.unspecified)
        let gap: CGFloat = 1.5
        let opTop = sup.height > 0 ? sup.height + gap : 0
        let opBaseline = subviews[1].dimensions(in: .unspecified)[.firstTextBaseline]
        let height = opTop + op.height + (sub.height > 0 ? gap + sub.height : 0)
        return (max(sup.width, op.width, sub.width), gap, sup, op, sub, opTop + opBaseline, height)
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let m = metrics(subviews); return CGSize(width: m.width, height: m.height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let m = metrics(subviews)
        let opTop = m.sup.height > 0 ? m.sup.height + m.gap : 0
        subviews[0].place(at: CGPoint(x: bounds.midX, y: bounds.minY), anchor: .top, proposal: .unspecified)
        subviews[1].place(at: CGPoint(x: bounds.midX, y: bounds.minY + opTop), anchor: .top, proposal: .unspecified)
        subviews[2].place(at: CGPoint(x: bounds.midX, y: bounds.minY + opTop + m.op.height + m.gap), anchor: .top, proposal: .unspecified)
    }

    func explicitAlignment(of guide: VerticalAlignment, in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGFloat? {
        guide == .firstTextBaseline || guide == .lastTextBaseline ? bounds.minY + metrics(subviews).baseline : nil
    }
}

private struct RadicalShape: Shape {
    func path(in rect: CGRect) -> Path {
        var p = Path()
        let sign = min(rect.width, rect.height * 0.6)
        p.move(to: CGPoint(x: 0, y: rect.height * 0.6))
        p.addLine(to: CGPoint(x: sign * 0.28, y: rect.height * 0.54))
        p.addLine(to: CGPoint(x: sign * 0.58, y: rect.height - 1))
        p.addLine(to: CGPoint(x: sign * 1.0, y: 0.5))
        p.addLine(to: CGPoint(x: rect.width, y: 0.5))
        return p
    }
}

/// A bracket that stretches to the height of what it wraps.
private struct DelimiterShape: Shape {
    let glyph: String
    let isLeft: Bool

    func path(in rect: CGRect) -> Path {
        var p = Path()
        let w = rect.width, h = rect.height
        func x(_ v: CGFloat) -> CGFloat { isLeft ? v : w - v }
        switch glyph {
        case "(", ")":
            p.move(to: CGPoint(x: x(w * 0.85), y: 0))
            p.addCurve(to: CGPoint(x: x(w * 0.85), y: h), control1: CGPoint(x: x(w * 0.05), y: h * 0.3), control2: CGPoint(x: x(w * 0.05), y: h * 0.7))
        case "[", "]":
            p.move(to: CGPoint(x: x(w * 0.9), y: 0.5)); p.addLine(to: CGPoint(x: x(w * 0.35), y: 0.5))
            p.addLine(to: CGPoint(x: x(w * 0.35), y: h - 0.5)); p.addLine(to: CGPoint(x: x(w * 0.9), y: h - 0.5))
        case "{", "}":
            p.move(to: CGPoint(x: x(w * 0.9), y: 0))
            p.addQuadCurve(to: CGPoint(x: x(w * 0.5), y: h * 0.1), control: CGPoint(x: x(w * 0.5), y: 0))
            p.addLine(to: CGPoint(x: x(w * 0.5), y: h * 0.4))
            p.addQuadCurve(to: CGPoint(x: x(w * 0.1), y: h * 0.5), control: CGPoint(x: x(w * 0.5), y: h * 0.5))
            p.addQuadCurve(to: CGPoint(x: x(w * 0.5), y: h * 0.6), control: CGPoint(x: x(w * 0.5), y: h * 0.5))
            p.addLine(to: CGPoint(x: x(w * 0.5), y: h * 0.9))
            p.addQuadCurve(to: CGPoint(x: x(w * 0.9), y: h), control: CGPoint(x: x(w * 0.5), y: h))
        case "|":
            p.move(to: CGPoint(x: w * 0.5, y: 0)); p.addLine(to: CGPoint(x: w * 0.5, y: h))
        case "‖":
            p.move(to: CGPoint(x: w * 0.3, y: 0)); p.addLine(to: CGPoint(x: w * 0.3, y: h))
            p.move(to: CGPoint(x: w * 0.7, y: 0)); p.addLine(to: CGPoint(x: w * 0.7, y: h))
        case "⟨", "⟩":
            p.move(to: CGPoint(x: x(w * 0.85), y: 0)); p.addLine(to: CGPoint(x: x(w * 0.15), y: h / 2)); p.addLine(to: CGPoint(x: x(w * 0.85), y: h))
        case "⌊", "⌋":
            p.move(to: CGPoint(x: x(w * 0.35), y: 0)); p.addLine(to: CGPoint(x: x(w * 0.35), y: h - 0.5)); p.addLine(to: CGPoint(x: x(w * 0.9), y: h - 0.5))
        case "⌈", "⌉":
            p.move(to: CGPoint(x: x(w * 0.9), y: 0.5)); p.addLine(to: CGPoint(x: x(w * 0.35), y: 0.5)); p.addLine(to: CGPoint(x: x(w * 0.35), y: h))
        default:
            p.move(to: CGPoint(x: w * 0.5, y: 0)); p.addLine(to: CGPoint(x: w * 0.5, y: h))
        }
        return p
    }
}

// MARK: - code blocks

/// A code block: a bar with the language and a Copy button (shown on hover), then the code in colour. The text can be
/// selected once the answer is finished; very long lines scroll sideways.
private struct CodeBlockView: View {
    let language: String?
    let code: String
    let live: Bool
    @State private var hovering = false
    @State private var copied = false

    private var name: String {
        let first = (language ?? "").split(whereSeparator: { $0 == " " || $0 == "{" }).first.map(String.init) ?? ""
        return first.isEmpty ? "código" : first.lowercased()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                Text(verbatim: name).font(.system(size: 9.5, weight: .medium)).foregroundColor(Ink.faint)
                Spacer(minLength: 0)
                if !live {
                    Button(action: copy) {
                        Text(verbatim: copied ? "Copiado ✓" : "Copiar")
                            .font(.system(size: 9.5, weight: .medium))
                            .foregroundColor(copied ? Ink.ok : Ink.body)
                            .padding(.horizontal, 6).padding(.vertical, 1)
                            .background(Capsule().fill(Color.white.opacity(0.08)))
                    }
                    .buttonStyle(.plain)
                    .opacity(hovering || copied ? 1 : 0)
                    .accessibilityLabel("Copiar el código")
                }
            }
            .padding(.horizontal, 9).padding(.vertical, 4)
            .background(Color.white.opacity(0.05))
            ScrollView(.horizontal, showsIndicators: false) {
                Text(CodeInk.attributed(code, language: language))
                    .font(.system(size: 11, design: .monospaced))
                    .lineSpacing(2)
                    .selectable(!live)
                    .fixedSize(horizontal: true, vertical: true)
                    .padding(.horizontal, 9).padding(.top, 6).padding(.bottom, 8)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.white.opacity(0.07))
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .onHover { hovering = $0 }
    }

    private func copy() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(code, forType: .string)
        copied = true
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: 1_500_000_000)
            copied = false
        }
    }
}
