import SwiftUI

/// GitHub-style calendar of recorded tokens, shared in meaning with the Windows calendar.
struct UsageCalendar: View {
    let days: [TokenDay]
    @State private var provider = "all"
    @State private var selected: Date?
    private let calendar = Calendar.current
    private let shades = ["#EBEDF0", "#9BE9A8", "#40C463", "#30A14E", "#216E39"]
    private let darkShades = ["#242A30", "#0E4429", "#006D32", "#26A641", "#39D353"]
    @Environment(\.colorScheme) private var scheme

    private var totals: [String: (tokens: Int64, turns: Int64)] {
        var result: [String: (tokens: Int64, turns: Int64)] = [:]
        for day in days where provider == "all" || provider == day.provider {
            let old = result[day.date] ?? (0, 0)
            result[day.date] = (old.tokens + day.input + day.output + day.cached, old.turns + day.turns)
        }
        return result
    }

    private func key(_ date: Date) -> String {
        let c = calendar.dateComponents([.year, .month, .day], from: date)
        return String(format: "%04d-%02d-%02d", c.year!, c.month!, c.day!)
    }

    private func color(_ level: Int) -> Color { Color(hex: (scheme == .dark ? darkShades : shades)[level]) }

    var body: some View {
        let today = calendar.startOfDay(for: Date())
        let sunday = calendar.date(byAdding: .day, value: -(calendar.component(.weekday, from: today) - 1), to: today)!
        let start = calendar.date(byAdding: .day, value: -364, to: sunday)!
        let values = totals.filter { $0.key >= key(start) && $0.key <= key(today) }
        let maximum = max(values.values.map(\.tokens).max() ?? 0, 1)
        let dates = (0..<371).map { calendar.date(byAdding: .day, value: $0, to: start)! }
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                VStack(alignment: .leading, spacing: 3) {
                    Text("Actividad de uso").font(.headline)
                    Text("Tokens por día · último año").font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Picker("Proveedor", selection: $provider) {
                    Text("Todos").tag("all")
                    Text("Claude").tag("claude")
                    Text("Codex").tag("codex")
                    Text("Gemini").tag("antigravity")
                }
                .labelsHidden().pickerStyle(.segmented).frame(width: 290)
            }
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(alignment: .top, spacing: 6) {
                    VStack(alignment: .leading, spacing: 2) {
                        Color.clear.frame(height: 14)
                        ForEach(0..<7) { row in
                            Text([1: "Lun", 3: "Mié", 5: "Vie"][row] ?? "")
                                .font(.system(size: 8)).foregroundStyle(.secondary).frame(width: 20, height: 8, alignment: .leading)
                        }
                    }
                    HStack(alignment: .top, spacing: 2) {
                        ForEach(0..<53) { week in
                            let monthDate = min(dates[week * 7 + 6], today)
                            VStack(spacing: 2) {
                                Text(week == 0 || calendar.component(.month, from: monthDate) != calendar.component(.month, from: dates[week * 7 - 1])
                                     ? monthDate.formatted(.dateTime.month(.abbreviated).locale(Locale(identifier: "es"))) : "")
                                    .font(.system(size: 9)).fixedSize().frame(width: 8, height: 14, alignment: .leading)
                                ForEach(0..<7) { row in
                                    let date = dates[week * 7 + row]
                                    let value = values[key(date)] ?? (tokens: 0, turns: 0)
                                    let level = value.tokens == 0 ? 0 : min(4, max(1, Int(ceil(Double(value.tokens) / Double(maximum) * 4))))
                                    Button { selected = date } label: {
                                        RoundedRectangle(cornerRadius: 2).fill(color(level)).frame(width: 8, height: 8)
                                    }
                                    .buttonStyle(.plain).disabled(date > today).opacity(date > today ? 0 : 1)
                                    .help(detail(date, value.tokens, value.turns))
                                    .accessibilityLabel(detail(date, value.tokens, value.turns))
                                }
                            }
                        }
                    }
                }
                .padding(.trailing, 16)
            }
            HStack {
                Text("\(UsageSettings.k(values.values.reduce(0) { $0 + $1.tokens })) tokens · \(values.values.reduce(0) { $0 + $1.turns }) turnos registrados")
                    .font(.caption).foregroundStyle(.secondary)
                Spacer()
                Text("Menos").font(.caption).foregroundStyle(.secondary)
                ForEach(0..<5) { level in RoundedRectangle(cornerRadius: 2).fill(color(level)).frame(width: 9, height: 9) }
                Text("Más").font(.caption).foregroundStyle(.secondary)
            }
            if let date = selected {
                let value = values[key(date)] ?? (tokens: 0, turns: 0)
                Text(detail(date, value.tokens, value.turns)).font(.caption).foregroundStyle(.secondary)
            } else {
                Text("Selecciona un día para ver su uso. Los días vacíos no tienen consumo registrado.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .padding(16)
        .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Color.secondary.opacity(0.2)))
    }

    private func detail(_ date: Date, _ tokens: Int64, _ turns: Int64) -> String {
        "\(date.formatted(.dateTime.day().month(.wide).year())) · \(tokens.formatted()) tokens · \(turns) turnos"
    }
}
