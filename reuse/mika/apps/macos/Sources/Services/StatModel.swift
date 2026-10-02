import Foundation

// MIKA's own numbers behind PARLEY's picks: probabilities computed here, from the statistics PARLEY found (with their
// sources), never from the model's judgement. Football: independent Poisson goals from each side's scoring and conceding
// rates. Tennis: a logistic on the ranking gap combined (log5) with each player's win rate on the surface. They are
// estimates for comparing against the price, not certainties; the page says so.

struct FootballModel: Equatable, Sendable {
    /// Expected goals of each side.
    var home: Double
    var away: Double
    /// P(home goals = i, away goals = j), i, j in 0...maxGoals.
    var grid: [[Double]]

    static let maxGoals = 10

    var homeWin: Double { sum { $0 > $1 } }
    var draw: Double { sum { $0 == $1 } }
    var awayWin: Double { sum { $0 < $1 } }
    var bothScore: Double { sum { $0 > 0 && $1 > 0 } }
    func over(_ line: Double) -> Double { sum { Double($0 + $1) > line } }

    /// P(total goals = n) for n in 0...5, then 6 or more.
    var totals: [Double] {
        var out = Array(repeating: 0.0, count: 7)
        for i in 0...Self.maxGoals { for j in 0...Self.maxGoals { out[min(i + j, 6)] += grid[i][j] } }
        return out
    }

    /// The three likeliest scores.
    var likelyScores: [(home: Int, away: Int, p: Double)] {
        var all: [(Int, Int, Double)] = []
        for i in 0...6 { for j in 0...6 { all.append((i, j, grid[i][j])) } }
        return all.sorted { $0.2 > $1.2 }.prefix(3).map { (home: $0.0, away: $0.1, p: $0.2) }
    }

    private func sum(_ keep: (Int, Int) -> Bool) -> Double {
        var total = 0.0
        for i in 0...Self.maxGoals { for j in 0...Self.maxGoals where keep(i, j) { total += grid[i][j] } }
        return total
    }
}

enum StatModel {
    /// Goals per match from each side's scoring rate and the other's conceding rate (both per match, ideally the home
    /// side at home and the away side away). Nil when a rate is missing or absurd.
    static func football(homeFor: Double?, homeAgainst: Double?, awayFor: Double?, awayAgainst: Double?) -> FootballModel? {
        guard let hf = homeFor, let ha = homeAgainst, let af = awayFor, let aa = awayAgainst,
              [hf, ha, af, aa].allSatisfy({ $0.isFinite && $0 >= 0 && $0 <= 8 }) else { return nil }
        let home = min(max((hf + aa) / 2, 0.05), 6)
        let away = min(max((af + ha) / 2, 0.05), 6)
        let ph = poisson(home), pa = poisson(away)
        var grid = Array(repeating: Array(repeating: 0.0, count: FootballModel.maxGoals + 1), count: FootballModel.maxGoals + 1)
        var total = 0.0
        for i in 0...FootballModel.maxGoals { for j in 0...FootballModel.maxGoals { grid[i][j] = ph[i] * pa[j]; total += grid[i][j] } }
        // What falls past ten goals is spread back so the grid adds up to one.
        if total > 0 { for i in grid.indices { for j in grid[i].indices { grid[i][j] /= total } } }
        return FootballModel(home: home, away: away, grid: grid)
    }

    static func poisson(_ lambda: Double) -> [Double] {
        var out = [exp(-lambda)]
        for k in 1...FootballModel.maxGoals { out.append(out[k - 1] * lambda / Double(k)) }
        return out
    }

    /// P(player A beats B). Ranking: logistic on the log-ranking gap (a player ranked half as high wins about 64 %).
    /// Surface: log5 of each player's win rate on it. Both when both are known, averaged; nil without either.
    static func tennis(rankA: Double?, rankB: Double?, surfaceA: Double?, surfaceB: Double?) -> Double? {
        var parts: [Double] = []
        if let a = rankA, let b = rankB, a >= 1, b >= 1, a < 5000, b < 5000 {
            parts.append(1 / (1 + exp(-0.85 * (log(b) - log(a)))))
        }
        if let a = surfaceA.map(rate), let b = surfaceB.map(rate), a > 0, a < 1, b > 0, b < 1 {
            parts.append(a * (1 - b) / (a * (1 - b) + b * (1 - a)))
        }
        guard !parts.isEmpty else { return nil }
        return min(max(parts.reduce(0, +) / Double(parts.count), 0.02), 0.98)
    }

    /// 0.68 or 68 → 0.68, clamped inside (0, 1).
    private static func rate(_ value: Double) -> Double {
        let v = value > 1 ? value / 100 : value
        return min(max(v, 0.01), 0.99)
    }

    // MARK: - Price arithmetic

    /// The bookmaker's price as a probability (1/odds, with its margin).
    static func implied(_ odds: Double?) -> Double? {
        guard let odds, odds > 1 else { return nil }
        return 1 / odds
    }

    /// The price at which a probability breaks even.
    static func fairOdds(_ probability: Double?) -> Double? {
        guard let p = probability, p > 0.001, p < 1 else { return nil }
        return 1 / p
    }

    /// Expected return per unit staked: p × odds − 1.
    static func edge(probability: Double?, odds: Double?) -> Double? {
        guard let p = probability, let odds, odds > 1, p > 0, p <= 1 else { return nil }
        return p * odds - 1
    }

    /// The model's probability for a market name, when the market is one the Poisson grid answers (1X2, totals, BTTS).
    static func probability(of market: String, home: String, away: String, model: FootballModel) -> Double? {
        let m = fold(market)
        // The grid is the whole match: half-time markets are not in it.
        let halves = ["descanso", "primer tiempo", "segundo tiempo", "medio tiempo", "1t", "2t", "1a mitad", "2a mitad", "primera mitad", "segunda mitad"]
        if halves.contains(where: { m.contains($0) }) { return nil }
        // A team is named in the market by its full name or by a word of it ("Correcaminos" for "Correcaminos UAT").
        let h = team(home, in: m, other: away) ? fold(home) : "\u{0}"
        let a = team(away, in: m, other: home) ? fold(away) : "\u{0}"
        if m.contains("ambos marcan") || m.contains("btts") {
            return m.contains("no") && !m.contains("si") ? 1 - model.bothScore : model.bothScore
        }
        if let line = goalLine(m) {
            if m.contains("mas de") || m.contains("over") || m.hasPrefix("+") { return model.over(line) }
            if m.contains("menos de") || m.contains("under") { return 1 - model.over(line) }
        }
        if m.contains("doble oportunidad") || ["1x", "x2", "12"].contains(m) {
            if m.contains("1x") || (h != "\u{0}" && m.contains("empate")) { return model.homeWin + model.draw }
            if m.contains("x2") || (a != "\u{0}" && m.contains("empate")) { return model.awayWin + model.draw }
            if m.hasSuffix("12") || (h != "\u{0}" && a != "\u{0}") { return model.homeWin + model.awayWin }
            return nil
        }
        if m == "empate" || m.hasPrefix("empate") || m == "x" { return model.draw }
        let winWords = m.contains("gana") || m.contains("ganador") || m.contains("victoria") || m == "1" || m == "2"
        if winWords && !m.contains("handicap") && !m.contains("hándicap") && !m.contains("tiempo") {
            if m == "1" || h != "\u{0}" || m.contains("local") { return model.homeWin }
            if m == "2" || a != "\u{0}" || m.contains("visita") { return model.awayWin }
        }
        return nil
    }

    static func fold(_ text: String) -> String {
        text.lowercased().folding(options: .diacriticInsensitive, locale: Locale(identifier: "es"))
    }

    /// The market names this team: its whole name, or a word of four letters or more that the other team's name lacks.
    static func team(_ name: String, in market: String, other: String) -> Bool {
        func words(_ text: String) -> [String] { text.split(whereSeparator: { !$0.isLetter && !$0.isNumber }).map(String.init) }
        let full = words(fold(name)).joined(separator: " ")
        guard !full.isEmpty else { return false }
        // The whole name as whole words ("g" is not in "belgica").
        if (" " + words(market).joined(separator: " ") + " ").contains(" " + full + " ") { return true }
        let otherWords = Set(fold(other).split(whereSeparator: { !$0.isLetter && !$0.isNumber }).map(String.init))
        let words = full.split(whereSeparator: { !$0.isLetter && !$0.isNumber }).map(String.init)
            .filter { $0.count >= 4 && !otherWords.contains($0) && !["club", "deportivo", "atletico", "united", "city", "real", "sporting"].contains($0) }
        let marketWords = Set(market.split(whereSeparator: { !$0.isLetter && !$0.isNumber }).map(String.init))
        return words.contains { marketWords.contains($0) }
    }

    /// "más de 2.5 goles" → 2.5. Only goal lines (a half number up to 7.5).
    private static func goalLine(_ market: String) -> Double? {
        guard market.contains("gol") || market.contains("over") || market.contains("under") else { return nil }
        let pattern = try? NSRegularExpression(pattern: #"(\d)[.,]5"#)
        guard let match = pattern?.firstMatch(in: market, range: NSRange(market.startIndex..., in: market)),
              let range = Range(match.range(at: 1), in: market), let whole = Double(market[range]) else { return nil }
        return whole + 0.5
    }
}
