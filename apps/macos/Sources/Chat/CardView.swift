import SwiftUI
import Charts

/// The card's measures and colours, from assets/design-tokens.json (`card`, `chart`, `color`, `colorLight`): the
/// same file the Windows app reads, so a card is the same card on both. Twin of `card.ts` / `.ui-card` on Windows.
struct CardStyle {
    var padding = 14.0, gap = 12.0, radius = 12.0, maxWidth = 380.0
    var titleSize = 14.0, labelSize = 11.0, bodySize = 13.0, metricSize = 20.0, heroSize = 40.0
    var chartHeight = 150.0, actionHeight = 26.0, sceneHeight = 120.0
    var dark: [String: String] = [:], light: [String: String] = [:]
    var chartDark: [String] = ["#6EE7A0"], chartLight: [String] = ["#1F9D57"]

    static let shared: CardStyle = {
        var style = CardStyle()
        guard let url = Bundle.main.url(forResource: "design-tokens", withExtension: "json"),
              let data = try? Data(contentsOf: url),
              let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return style }
        let card = root["card"] as? [String: Double] ?? [:]
        func number(_ key: String, _ fallback: Double) -> Double { card[key] ?? fallback }
        style.padding = number("padding", style.padding)
        style.gap = number("gap", style.gap)
        style.radius = number("radius", style.radius)
        style.maxWidth = number("maxWidth", style.maxWidth)
        style.titleSize = number("titleSize", style.titleSize)
        style.labelSize = number("labelSize", style.labelSize)
        style.bodySize = number("bodySize", style.bodySize)
        style.metricSize = number("metricSize", style.metricSize)
        style.heroSize = number("heroSize", style.heroSize)
        style.chartHeight = number("chartHeight", style.chartHeight)
        style.actionHeight = number("actionHeight", style.actionHeight)
        style.sceneHeight = number("sceneHeight", style.sceneHeight)
        style.dark = root["color"] as? [String: String] ?? [:]
        style.light = root["colorLight"] as? [String: String] ?? [:]
        let chart = root["chart"] as? [String: [String]] ?? [:]
        style.chartDark = chart["dark"] ?? style.chartDark
        style.chartLight = chart["light"] ?? style.chartLight
        return style
    }()

    func color(_ name: String) -> Color {
        Color.dynamic(light: light[name] ?? "#09090B", dark: dark[name] ?? "#FAFAFA")
    }

    func series(_ index: Int) -> Color {
        Color.dynamic(light: chartLight[index % chartLight.count], dark: chartDark[index % chartDark.count])
    }
}

/// What a card's follow-up button does: sends that message for the user.
private struct SendPromptKey: EnvironmentKey {
    static let defaultValue: @MainActor @Sendable (String) -> Void = { _ in }
}

extension EnvironmentValues {
    var sendPrompt: @MainActor @Sendable (String) -> Void {
        get { self[SendPromptKey.self] }
        set { self[SendPromptKey.self] = newValue }
    }
}

/// One card of an answer (see `core/src/cards.rs`): title, figures, chart, table, steps, weather and follow-ups, in
/// that order, each only when the card has it.
struct CardView: View {
    let card: Card
    var interactive = true
    /// The card is still arriving: what it has so far is shown, and a skeleton stands for the rest.
    var drawing = false
    @Environment(\.sendPrompt) private var sendPrompt
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private let style = CardStyle.shared

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // The weather's sky runs edge to edge at the top; everything else sits inside the card's padding.
            if let weather = card.weather { WeatherHero(weather: weather, style: style) }
            VStack(alignment: .leading, spacing: style.gap) {
                if !card.title.isEmpty || !card.subtitle.isEmpty { header.transition(Self.enter) }
                if let weather = card.weather { WeatherDetails(weather: weather, style: style).transition(Self.enter) }
                if !card.metrics.isEmpty { metrics.transition(Self.enter) }
                if let chart = card.chart { ChartPanel(chart: chart, style: style).transition(Self.enter) }
                if let table = card.table { TablePanel(table: table, style: style).transition(Self.enter) }
                if !card.steps.isEmpty { steps.transition(Self.enter) }
                if drawing { SkeletonLines().transition(.opacity) }
                if !card.actions.isEmpty && interactive { actions.transition(Self.enter) }
                if !card.source.isEmpty || !card.drawnBy.isEmpty { footer.transition(Self.enter) }
            }
            .padding(style.padding)
        }
        .frame(maxWidth: style.maxWidth, alignment: .leading)
        .background(style.color("surface"))
        .clipShape(RoundedRectangle(cornerRadius: style.radius, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: style.radius, style: .continuous).strokeBorder(style.color("stroke"), lineWidth: 1))
        // Each part comes in as it arrives (and the bars grow), instead of the whole card landing at once.
        .animation(reduceMotion ? nil : .easeOut(duration: 0.25), value: card)
        .animation(reduceMotion ? nil : .easeOut(duration: 0.25), value: drawing)
        .accessibilityElement(children: .contain)
    }

    /// How a part of the card comes in. Twin of `.ui-enter` on Windows.
    private static let enter = AnyTransition.opacity.combined(with: .offset(y: 4))

    /// Where the data comes from, and the model that drew the card (when one did).
    private var footer: some View {
        HStack(spacing: 8) {
            if !card.source.isEmpty {
                Text("Fuente: \(card.source)").lineLimit(1).truncationMode(.tail)
            }
            Spacer(minLength: 0)
            if !card.drawnBy.isEmpty {
                Text(card.drawnBy).lineLimit(1).tip("Tarjeta dibujada por \(card.drawnBy)")
            }
        }
        .font(.system(size: style.labelSize - 1))
        .foregroundStyle(style.color("textMuted"))
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 2) {
            if !card.title.isEmpty {
                Text(card.title).font(.system(size: style.titleSize, weight: .semibold)).foregroundStyle(style.color("text"))
            }
            if !card.subtitle.isEmpty {
                Text(card.subtitle).font(.system(size: style.labelSize + 1)).foregroundStyle(style.color("textMuted"))
            }
        }
    }

    private var metrics: some View {
        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: style.gap, alignment: .leading), count: min(card.metrics.count, 3)),
                  alignment: .leading, spacing: style.gap) {
            ForEach(Array(card.metrics.enumerated()), id: \.offset) { _, metric in
                VStack(alignment: .leading, spacing: 2) {
                    Text(metric.label).font(.system(size: style.labelSize, weight: .medium)).foregroundStyle(style.color("textMuted")).lineLimit(1)
                    Text(metric.value).font(.system(size: style.metricSize, weight: .semibold)).monospacedDigit()
                        .foregroundStyle(style.color("text")).lineLimit(1).minimumScaleFactor(0.6)
                    if !metric.note.isEmpty {
                        Text(metric.note).font(.system(size: style.labelSize)).foregroundStyle(tone(metric.tone)).lineLimit(1)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }

    private func tone(_ tone: String) -> Color {
        switch tone {
        case "bien": style.color("success")
        case "mal": style.color("danger")
        case "aviso": style.color("warning")
        default: style.color("textMuted")
        }
    }

    private var steps: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(card.steps.enumerated()), id: \.offset) { index, step in
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text("\(index + 1)")
                        .font(.system(size: style.labelSize, weight: .semibold)).monospacedDigit()
                        .foregroundStyle(style.color("textMuted"))
                        .frame(width: 18, height: 18)
                        .background(style.color("surfaceRaised"), in: Circle())
                    Text(step).font(.system(size: style.bodySize)).foregroundStyle(style.color("text")).fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }

    private var actions: some View {
        FlowRow(spacing: 6) {
            ForEach(card.actions, id: \.self) { action in
                Button { sendPrompt(action) } label: {
                    Text(action)
                        .font(.system(size: style.labelSize + 1, weight: .medium))
                        .foregroundStyle(style.color("text"))
                        .lineLimit(1)
                        .padding(.horizontal, 10)
                        .frame(height: style.actionHeight)
                        .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(style.color("stroke"), lineWidth: 1))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .tip("Enviar: \(action)")
            }
        }
    }
}

/// A grey block that stands for something still to come. It breathes while it waits (not with reduced motion).
/// Twin of `.ui-skel` on Windows.
private struct SkeletonBlock: View {
    var width: CGFloat? = nil
    let height: CGFloat
    @State private var dim = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        RoundedRectangle(cornerRadius: 4, style: .continuous)
            .fill(CardStyle.shared.color("stroke"))
            .frame(width: width, height: height)
            .frame(maxWidth: width == nil ? .infinity : nil)
            .opacity(dim ? 0.5 : 1)
            .onAppear {
                guard !reduceMotion else { return }
                withAnimation(.easeInOut(duration: 0.8).repeatForever(autoreverses: true)) { dim = true }
            }
    }
}

/// Two lines under what a card already has: more is on its way.
private struct SkeletonLines: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            SkeletonBlock(width: 180, height: 9)
            SkeletonBlock(width: 110, height: 9)
        }
        .accessibilityHidden(true)
    }
}

/// A card that has opened but has nothing to show yet: a title, three figures and a chart in grey blocks, so it is
/// clear that something is being drawn. Twin of `cardSkeleton` on Windows.
struct CardSkeleton: View {
    private let style = CardStyle.shared

    var body: some View {
        VStack(alignment: .leading, spacing: style.gap) {
            VStack(alignment: .leading, spacing: 6) {
                SkeletonBlock(width: 150, height: 12)
                SkeletonBlock(width: 100, height: 9)
            }
            HStack(spacing: style.gap) {
                ForEach(0..<3, id: \.self) { _ in
                    VStack(alignment: .leading, spacing: 5) {
                        SkeletonBlock(width: 44, height: 8)
                        SkeletonBlock(width: 72, height: 16)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            HStack(alignment: .bottom, spacing: 10) {
                ForEach([0.45, 0.8, 0.6, 1.0, 0.4, 0.7], id: \.self) { part in
                    SkeletonBlock(height: 84 * part)
                }
            }
            .frame(height: 84, alignment: .bottom)
        }
        .padding(style.padding)
        .frame(maxWidth: style.maxWidth, alignment: .leading)
        .background(style.color("surface"))
        .clipShape(RoundedRectangle(cornerRadius: style.radius, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: style.radius, style: .continuous).strokeBorder(style.color("stroke"), lineWidth: 1))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Dibujando la tarjeta")
    }
}

/// A weather picture as an SF Symbol (the days' row).
private func weatherSymbol(_ icon: String) -> String {
    switch icon {
    case "sol": "sun.max"
    case "parcial": "cloud.sun"
    case "niebla": "cloud.fog"
    case "lluvia": "cloud.rain"
    case "tormenta": "cloud.bolt.rain"
    case "nieve": "cloud.snow"
    default: "cloud"
    }
}

/// The top of a weather card: the place, the temperature and the sky's words, over that sky (`weather_backdrop`).
private struct WeatherHero: View {
    let weather: Weather
    let style: CardStyle

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(weather.place).font(.system(size: style.labelSize + 1, weight: .medium)).opacity(0.9).lineLimit(1)
            Text("\(Int(weather.temperature.rounded()))°").font(.system(size: style.heroSize, weight: .semibold)).monospacedDigit()
            Text("\(weather.condition) · ↑\(Int(weather.high.rounded()))° ↓\(Int(weather.low.rounded()))°")
                .font(.system(size: style.bodySize, weight: .medium)).monospacedDigit().lineLimit(1)
        }
        .foregroundStyle(.white)
        .shadow(color: .black.opacity(0.35), radius: 2, y: 1)
        .padding(style.padding)
        .frame(maxWidth: .infinity, minHeight: style.sceneHeight, alignment: .leading)
        .background { BackdropView(backdrop: weatherBackdrop(icon: weather.icon, night: weather.night)) }
        .clipped()
        .accessibilityElement(children: .combine)
    }
}

/// Paints a backdrop from the core's shapes, covering the space and keeping its right edge. Twin of `backdropView`
/// on Windows (`preserveAspectRatio="xMaxYMid slice"`).
struct BackdropView: View {
    let backdrop: Backdrop

    var body: some View {
        Canvas { context, size in
            let scale = max(size.width / backdrop.width, size.height / backdrop.height)
            context.translateBy(x: size.width - backdrop.width * scale, y: (size.height - backdrop.height * scale) / 2)
            context.scaleBy(x: scale, y: scale)
            for shape in backdrop.shapes {
                var layer = context
                layer.opacity = shape.opacity
                let color = Color(hex: shape.color)
                let box = CGRect(x: shape.x, y: shape.y, width: shape.w, height: shape.h)
                switch shape.kind {
                case "ellipse":
                    layer.fill(Path(ellipseIn: box), with: .color(color))
                case "line":
                    var path = Path()
                    path.move(to: box.origin)
                    path.addLine(to: CGPoint(x: shape.x + shape.w, y: shape.y + shape.h))
                    layer.stroke(path, with: .color(color), style: StrokeStyle(lineWidth: shape.size, lineCap: .round))
                case "poly":
                    var path = Path()
                    for i in stride(from: 0, to: shape.points.count - 1, by: 2) {
                        let point = CGPoint(x: shape.points[i], y: shape.points[i + 1])
                        if i == 0 { path.move(to: point) } else { path.addLine(to: point) }
                    }
                    path.closeSubpath()
                    layer.fill(path, with: .color(color))
                default:
                    let path = Path(roundedRect: box, cornerRadius: shape.size)
                    if shape.color2.isEmpty {
                        layer.fill(path, with: .color(color))
                    } else {
                        layer.fill(path, with: .linearGradient(Gradient(colors: [color, Color(hex: shape.color2)]),
                                                               startPoint: box.origin, endPoint: CGPoint(x: box.minX, y: box.maxY)))
                    }
                }
            }
        }
        .accessibilityHidden(true)
    }
}

/// Under the sky: today's facts and the next days.
private struct WeatherDetails: View {
    let weather: Weather
    let style: CardStyle

    /// The chance of rain shows under each day only when some day is worth it.
    private var showsRain: Bool { weather.days.contains { $0.rain >= 30 } }

    var body: some View {
        VStack(alignment: .leading, spacing: style.gap) {
            HStack(spacing: 0) {
                fact("Sensación", "\(Int(weather.feelsLike.rounded()))°")
                fact("Humedad", "\(Int(weather.humidity.rounded())) %")
                fact("Viento", "\(Int(weather.wind.rounded())) km/h")
                fact("Lluvia", "\(Int(weather.rain.rounded())) %")
            }
            if !weather.days.isEmpty {
                Rectangle().fill(style.color("stroke")).frame(height: 1)
                HStack(spacing: 0) {
                    ForEach(Array(weather.days.enumerated()), id: \.offset) { _, day in
                        VStack(spacing: 4) {
                            Text(day.label).font(.system(size: style.labelSize, weight: .medium)).foregroundStyle(style.color("textMuted"))
                            Image(systemName: weatherSymbol(day.icon)).font(.system(size: 15)).foregroundStyle(style.color("text")).frame(height: 18)
                            Text("\(Int(day.high.rounded()))°").font(.system(size: style.labelSize + 1, weight: .semibold)).monospacedDigit().foregroundStyle(style.color("text"))
                            Text("\(Int(day.low.rounded()))°").font(.system(size: style.labelSize)).monospacedDigit().foregroundStyle(style.color("textMuted"))
                            if showsRain {
                                Text(day.rain >= 30 ? "\(Int(day.rain.rounded())) %" : " ")
                                    .font(.system(size: style.labelSize - 1, weight: .medium)).monospacedDigit().foregroundStyle(style.series(4))
                            }
                        }
                        .frame(maxWidth: .infinity)
                    }
                }
            }
        }
        .accessibilityElement(children: .combine)
    }

    private func fact(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(label).font(.system(size: style.labelSize - 1, weight: .medium)).foregroundStyle(style.color("textMuted"))
            Text(value).font(.system(size: style.labelSize + 1, weight: .semibold)).monospacedDigit().foregroundStyle(style.color("text"))
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct ChartPanel: View {
    let chart: Buddy.Chart
    let style: CardStyle

    private struct Point: Identifiable {
        let id: Int
        let label: String
        let series: String
        let seriesIndex: Int
        let value: Double
    }

    private var points: [Point] {
        var out: [Point] = []
        for (s, series) in chart.series.enumerated() {
            for (i, label) in chart.labels.enumerated() where i < series.values.count {
                out.append(Point(id: out.count, label: label, series: series.name, seriesIndex: s, value: series.values[i]))
            }
        }
        return out
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if chart.style == "torta" { pie } else { plot }
            if chart.style != "torta" && chart.series.count > 1 { legend(chart.series.map(\.name)) }
        }
    }

    private var plot: some View {
        Charts.Chart(points) { point in
            if chart.style == "lineas" {
                LineMark(x: .value("", point.label), y: .value("", point.value))
                    .foregroundStyle(style.series(point.seriesIndex))
                    .foregroundStyle(by: .value("", point.series))
                    .interpolationMethod(.monotone)
                    .lineStyle(StrokeStyle(lineWidth: 2))
            } else {
                BarMark(x: .value("", point.label), y: .value("", point.value))
                    .foregroundStyle(style.series(point.seriesIndex))
                    .position(by: .value("", point.series))
                    .cornerRadius(3)
            }
        }
        .chartForegroundStyleScale(domain: chart.series.map(\.name), range: chart.series.indices.map { style.series($0) })
        .chartLegend(.hidden)
        .chartXAxis {
            AxisMarks { _ in
                AxisValueLabel().font(.system(size: style.labelSize - 1)).foregroundStyle(style.color("textMuted"))
            }
        }
        .chartYAxis {
            AxisMarks(position: .leading, values: .automatic(desiredCount: 4)) { _ in
                AxisGridLine(stroke: StrokeStyle(lineWidth: 1)).foregroundStyle(style.color("stroke"))
                AxisValueLabel().font(.system(size: style.labelSize - 1)).foregroundStyle(style.color("textMuted"))
            }
        }
        .frame(height: style.chartHeight)
        .accessibilityLabel("Gráfico \(chart.unit)")
    }

    private var pie: some View {
        let values = chart.series.first?.values ?? []
        return HStack(alignment: .center, spacing: 14) {
            Charts.Chart(Array(chart.labels.enumerated()), id: \.offset) { index, label in
                SectorMark(angle: .value(label, index < values.count ? values[index] : 0), innerRadius: .ratio(0.58), angularInset: 1)
                    .foregroundStyle(style.series(index))
            }
            .frame(width: style.chartHeight - 30, height: style.chartHeight - 30)
            VStack(alignment: .leading, spacing: 4) {
                ForEach(Array(chart.labels.enumerated()), id: \.offset) { index, label in
                    HStack(spacing: 6) {
                        Circle().fill(style.series(index)).frame(width: 7, height: 7)
                        Text(label).font(.system(size: style.labelSize + 1)).foregroundStyle(style.color("text")).lineLimit(1)
                        Spacer(minLength: 4)
                        Text(figure(index < values.count ? values[index] : 0)).font(.system(size: style.labelSize + 1)).monospacedDigit().foregroundStyle(style.color("textMuted"))
                    }
                }
            }
        }
    }

    private func legend(_ names: [String]) -> some View {
        HStack(spacing: 12) {
            ForEach(Array(names.enumerated()), id: \.offset) { index, name in
                HStack(spacing: 5) {
                    Circle().fill(style.series(index)).frame(width: 7, height: 7)
                    Text(name).font(.system(size: style.labelSize)).foregroundStyle(style.color("textMuted"))
                }
            }
        }
    }

    /// «S/. 1,224» or «45 %»: a currency sign goes before the figure, any other unit after it.
    private func figure(_ value: Double) -> String {
        let number = value.formatted(.number.precision(.fractionLength(0...1)).locale(Locale(identifier: "en_US")))
        if chart.unit.isEmpty { return number }
        return chart.unit.contains("$") || chart.unit.contains("S/") || chart.unit.contains("€") ? "\(chart.unit) \(number)" : "\(number) \(chart.unit)"
    }
}

private struct TablePanel: View {
    let table: Buddy.Table
    let style: CardStyle

    var body: some View {
        Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 0) {
            GridRow {
                ForEach(Array(table.columns.enumerated()), id: \.offset) { _, column in
                    Text(column).font(.system(size: style.labelSize, weight: .medium)).foregroundStyle(style.color("textMuted")).lineLimit(1)
                        .padding(.vertical, 6)
                }
            }
            ForEach(Array(table.rows.enumerated()), id: \.offset) { _, row in
                Rectangle().fill(style.color("stroke")).frame(height: 1).gridCellColumns(max(table.columns.count, 1))
                GridRow {
                    ForEach(Array(row.enumerated()), id: \.offset) { _, cell in
                        Text(cell).font(.system(size: style.bodySize - 1)).monospacedDigit().foregroundStyle(style.color("text"))
                            .lineLimit(2).fixedSize(horizontal: false, vertical: true)
                            .padding(.vertical, 6)
                    }
                }
            }
        }
    }
}

/// Buttons in rows that wrap, like `flex-wrap` on Windows.
private struct FlowRow: Layout {
    var spacing: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(width: proposal.width ?? .infinity, subviews: subviews)
        return CGSize(width: proposal.width ?? rows.map(\.width).max() ?? 0, height: rows.last.map { $0.y + $0.height } ?? 0)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        for row in arrange(width: bounds.width, subviews: subviews) {
            for item in row.items {
                subviews[item.index].place(at: CGPoint(x: bounds.minX + item.x, y: bounds.minY + row.y), proposal: .unspecified)
            }
        }
    }

    private struct Row { var y: CGFloat; var height: CGFloat; var width: CGFloat; var items: [(index: Int, x: CGFloat)] }

    private func arrange(width: CGFloat, subviews: Subviews) -> [Row] {
        var rows: [Row] = []
        for (index, view) in subviews.enumerated() {
            let size = view.sizeThatFits(.unspecified)
            if var last = rows.last, last.width + spacing + size.width <= width {
                last.items.append((index, last.width + spacing))
                last.width += spacing + size.width
                last.height = max(last.height, size.height)
                rows[rows.count - 1] = last
            } else {
                let y = rows.last.map { $0.y + $0.height + spacing } ?? 0
                rows.append(Row(y: y, height: size.height, width: size.width, items: [(index, 0)]))
            }
        }
        return rows
    }
}
