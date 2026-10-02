import Foundation

// PARLEY's daily digest: one page with every football and tennis match of the day, what the form and the web say about
// each, the Betano prices MIKA already holds, and PARLEY's picks, as tables. PARLEY writes the content as JSON; MIKA
// draws the page (all text escaped, links only http/https), saves it in PARLEY's workspace (`digest/<day>.html`) and
// opens it in the browser on a click. The scheduler and the notification are in PicksService.swift.

struct DigestMarket: Equatable, Codable, Sendable {
    var name: String
    /// PARLEY's probability (0–1).
    var prob: Double?
    /// The Betano price, when MIKA's tables had one.
    var odds: Double?
    var note: String = ""
    /// MIKA's own model (Poisson / ranking), when the market is one it answers. Computed, never read from PARLEY.
    var model: Double?
}

/// A trend behind a pick, Scout-style: "San Antonio: menos de 2.5 goles en 8 de sus últimos 10 en casa" (8 of 10).
struct DigestTrend: Equatable, Codable, Sendable {
    var text: String
    var hits: Int?
    var of: Int?
}

struct DigestMatchPick: Equatable, Codable, Sendable {
    var market: String
    var prob: Double?
    var odds: Double?
    var confidence: String
    var reason: String
    var model: Double?
}

struct DigestMatch: Equatable, Codable, Sendable {
    var sport: String        // "futbol" | "tenis"
    var tournament: String
    var match: String
    var home: String
    var away: String
    var time: String         // "13:45", local
    var odds: String         // "1 @1.67 · X @3.80 · 2 @4.00", "" when none
    var formHome: String = ""
    var formAway: String = ""
    var stats: [String] = []
    var absences: String = ""
    var lineups: String = ""
    var h2h: String = ""
    // The numbers MIKA's model runs on (football: goals per match; tennis: ranking and surface win rate).
    var goalsForHome: Double?
    var goalsAgainstHome: Double?
    var goalsForAway: Double?
    var goalsAgainstAway: Double?
    var rankHome: Double?
    var rankAway: Double?
    var surface: String = ""
    var surfaceHome: Double?
    var surfaceAway: Double?
    var markets: [DigestMarket] = []
    var pick: DigestMatchPick?
    var sources: [String] = []
    var trends: [DigestTrend] = []
    /// "2026-10-02" (the day of the match), and "fuerte" for the leagues analysed in depth.
    var date: String = ""
    var level: String = ""
    // MIKA's model, computed in `enrich`.
    var model1X2: [Double]?      // home, draw, away
    var modelTotals: [Double]?   // P(total goals = 0…5, 6+)
    var modelScores: [String]?   // "2-1 · 12 %"
    var modelTennis: Double?     // P(home player wins)
}

struct DigestPick: Equatable, Codable, Sendable {
    var kind: String         // "simple"
    var match: String
    var market: String
    var prob: Double?
    var odds: Double?
    var stake: String
    var confidence: String
    var reason: String
    var source: String
    var model: Double?
}

struct DigestCombo: Equatable, Codable, Sendable {
    var legs: [String]
    var prob: Double?
    var odds: Double?
    var stake: String
    var reason: String
}

/// A pick a Telegram channel posted, and what PARLEY makes of it.
struct DigestTip: Equatable, Codable, Sendable {
    var channel: String
    var author: String
    var time: String
    var pick: String
    var odds: Double?
    var evidence: String     // texto | captura | enlace
    var prob: Double?
    var verdict: String      // a favor | dudoso | en contra
    var reason: String
}

struct Digest: Equatable, Codable, Sendable {
    var day: String
    var generated: String    // "07:20"
    var summary: String
    var method: String = ""
    var matches: [DigestMatch]
    var picks: [DigestPick]
    var combos: [DigestCombo] = []
    var tips: [DigestTip] = []
    var warnings: [String] = []
}

enum ParleyDigest {
    /// The hour (local) from which the day's digest is made, by default: after the 06:30 snapshot of the day's prices, while
    /// most of the day's matches have not started yet (a digest at 11:00 found every priced match already played).
    static let defaultHour = 6
    /// A digest that fails waits this long before the next attempt.
    static let retry: TimeInterval = 20 * 60
    static let maxMatches = 150
    static let maxPicks = 20
    /// Telegram screenshots attached to the turn.
    static let maxImages = 16
    static let maxPosts = 120

    static func dir(workspace: URL) -> URL { workspace.appendingPathComponent("digest", isDirectory: true) }
    static func htmlFile(workspace: URL, day: String) -> URL { dir(workspace: workspace).appendingPathComponent("\(day).html") }
    static func jsonFile(workspace: URL, day: String) -> URL { dir(workspace: workspace).appendingPathComponent("\(day).json") }

    /// The day to make a digest for, or nil: not yet the hour, or already made (`marker` is the text of digest/last.json).
    static func due(now: Date, timeZone: TimeZone, hour: Int, marker: String?) -> String? {
        let day = ChatArchive.dateLabel(now, timeZone: timeZone)
        let minutes = Int(Picks.hour(now, timeZone: timeZone).prefix(2)) ?? 0
        guard minutes >= min(max(hour, 0), 23) else { return nil }
        if let marker, marker.contains("\"day\":\"\(day)\"") { return nil }
        return day
    }

    /// Unix seconds of the local midnight that starts the day of `now`.
    static func startOfDay(_ now: Date, timeZone: TimeZone) -> Int64 {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = timeZone
        return Int64(calendar.startOfDay(for: now).timeIntervalSince1970)
    }

    // MARK: - The question

    /// Today's Telegram posts as PARLEY reads them: channel, who wrote it, time, text, links and the path of each screenshot.
    static func postsText(_ posts: [TelegramPost], timeZone: TimeZone, media: URL) -> String {
        var out = ""
        for post in posts {
            let text = String(post.text.prefix(900)).replacingOccurrences(of: "\n", with: " ")
            let who = post.sender.isEmpty ? "" : " · autor: \(post.sender)"
            out += "- [canal: \(post.chat)\(who) · \(Picks.hour(post.date, timeZone: timeZone))] \(text.isEmpty ? "(sin texto)" : text)\n"
            if !post.links.isEmpty { out += "  enlaces: \(post.links.prefix(6).joined(separator: " "))\n" }
            for photo in post.photos where TelegramInbox.safeMediaName(photo) {
                out += "  captura: \(media.appendingPathComponent(photo).path)\n"
            }
        }
        return out
    }

    // MARK: - Its answer

    /// The JSON PARLEY wrote (fenced or bare), read tolerantly: a field of the wrong type is skipped, never an error, and
    /// an answer cut short is closed at its last complete value. Nil when there is no match and no pick.
    static func parse(_ answer: String, day: String, generated: String) -> Digest? {
        guard let start = answer.firstIndex(of: "{") else { return nil }
        let body = String(answer[start...])
        let value: JSONValue
        if let whole = decode(trimmedToLastBrace(body)) { value = whole }
        else if let repaired = repairTruncated(body).flatMap(decode) { value = repaired }
        else { return nil }
        guard case .object = value else { return nil }

        func text(_ v: JSONValue, _ key: String, _ max: Int = 240) -> String {
            let raw = v[key].stringValue ?? ""
            // Sources go in their own field: a link written in the text keeps only its words.
            return Picks.clean(key == "fuente" || key == "fuentes" ? raw : plain(raw), max: max)
        }
        func strings(_ v: JSONValue, _ key: String, count: Int, max: Int = 240) -> [String] {
            guard case .array(let items) = v[key] else { return [] }
            return Array(items.compactMap(\.stringValue).map { Picks.clean(key == "fuentes" ? $0 : plain($0), max: max) }
                .filter { !$0.isEmpty }.prefix(count))
        }

        var matches: [DigestMatch] = []
        if case .array(let items) = value["partidos"] {
            for item in items.prefix(maxMatches) {
                var home = text(item, "local", 80), away = text(item, "visita", 80)
                var name = text(item, "partido", 140)
                if name.isEmpty, !home.isEmpty, !away.isEmpty { name = "\(home) vs \(away)" }
                if home.isEmpty || away.isEmpty, let range = name.range(of: " vs ", options: .caseInsensitive) {
                    home = String(name[..<range.lowerBound]); away = String(name[range.upperBound...])
                }
                guard !name.isEmpty else { continue }
                let data = item["datos"]
                var m = DigestMatch(sport: text(item, "deporte", 20).lowercased().hasPrefix("t") ? "tenis" : "futbol",
                                    tournament: text(item, "torneo", 80), match: name, home: home, away: away,
                                    time: text(item, "hora", 12), odds: text(item, "cuotas", 200))
                m.formHome = Self.form(text(data, "forma_local", 20)); m.formAway = Self.form(text(data, "forma_visita", 20))
                m.h2h = text(data, "h2h", 120)
                m.goalsForHome = number(data["gf_local"]); m.goalsAgainstHome = number(data["gc_local"])
                m.goalsForAway = number(data["gf_visita"]); m.goalsAgainstAway = number(data["gc_visita"])
                m.rankHome = number(data["ranking_local"]); m.rankAway = number(data["ranking_visita"])
                m.surface = text(data, "superficie", 30)
                m.surfaceHome = number(data["pct_superficie_local"]); m.surfaceAway = number(data["pct_superficie_visita"])
                m.stats = strings(item, "estadisticas", count: 8)
                m.absences = text(item, "bajas", 300); m.lineups = text(item, "alineaciones", 300)
                if case .array(let markets) = item["mercados"] {
                    m.markets = markets.prefix(10).compactMap { mk in
                        let n = text(mk, "mercado", 100)
                        return n.isEmpty ? nil : DigestMarket(name: n, prob: probability(mk["prob"]), odds: odds(mk["cuota"]), note: text(mk, "nota", 200))
                    }
                }
                let pk = item["pick"]
                if !text(pk, "mercado", 100).isEmpty {
                    m.pick = DigestMatchPick(market: text(pk, "mercado", 100), prob: probability(pk["prob"]), odds: odds(pk["cuota"]),
                                             confidence: confidence(text(pk, "confianza", 12)), reason: text(pk, "motivo", 300))
                }
                m.sources = strings(item, "fuentes", count: 6, max: 300)
                m.date = text(item, "fecha", 10)
                m.level = text(item, "nivel", 12).lowercased().hasPrefix("f") ? "fuerte" : ""
                if case .array(let list) = item["tendencias"] {
                    m.trends = list.prefix(6).compactMap { t in
                        let words = text(t, "texto", 200)
                        guard !words.isEmpty else { return nil }
                        let hits = number(t["aciertos"]).map { Int($0) }, of = number(t["de"]).map { Int($0) }
                        let valid = hits != nil && of != nil && of! > 0 && hits! >= 0 && hits! <= of!
                        return DigestTrend(text: words, hits: valid ? hits : nil, of: valid ? of : nil)
                    }
                }
                matches.append(m)
            }
        }
        var picks: [DigestPick] = []
        if case .array(let items) = value["picks"] {
            for item in items.prefix(maxPicks) {
                let match = text(item, "partido", 140)
                guard !match.isEmpty else { continue }
                picks.append(DigestPick(kind: "simple", match: match, market: text(item, "mercado", 140), prob: probability(item["prob"]),
                                        odds: odds(item["cuota"]), stake: text(item, "monto", 30),
                                        confidence: confidence(text(item, "confianza", 12)), reason: text(item, "motivo", 320),
                                        source: text(item, "fuente", 300)))
            }
        }
        var combos: [DigestCombo] = []
        if case .array(let items) = value["combinadas"] {
            for item in items.prefix(6) {
                let legs = strings(item, "selecciones", count: 4, max: 140)
                guard legs.count >= 2 else { continue }
                combos.append(DigestCombo(legs: legs, prob: probability(item["prob"]), odds: odds(item["cuota"]),
                                          stake: text(item, "monto", 30), reason: text(item, "motivo", 300)))
            }
        }
        var tips: [DigestTip] = []
        if case .array(let items) = value["telegram"] {
            for item in items.prefix(40) {
                let pick = text(item, "pick", 160)
                guard !pick.isEmpty else { continue }
                let verdict = text(item, "veredicto", 20).lowercased()
                tips.append(DigestTip(channel: text(item, "canal", 60), author: text(item, "autor", 60), time: text(item, "hora", 12),
                                      pick: pick, odds: odds(item["cuota"]), evidence: text(item, "evidencia", 20).lowercased(),
                                      prob: probability(item["prob"]),
                                      verdict: verdict.contains("contra") ? "en contra" : verdict.contains("favor") ? "a favor" : "dudoso",
                                      reason: text(item, "motivo", 300)))
            }
        }
        guard !matches.isEmpty || !picks.isEmpty || !tips.isEmpty else { return nil }
        var digest = Digest(day: day, generated: generated, summary: text(value, "resumen", 600), method: text(value, "metodo", 400),
                            matches: matches, picks: picks, combos: combos, tips: tips, warnings: strings(value, "ojo", count: 8))
        enrich(&digest)
        return digest
    }

    /// MIKA's model next to PARLEY's numbers: the Poisson grid of each football match with goal rates, the ranking and
    /// surface estimate of each tennis match, and that model's probability for every market and pick it can answer.
    static func enrich(_ digest: inout Digest) {
        var byName: [String: FootballModel] = [:]
        var tennisByName: [String: (home: String, away: String, p: Double)] = [:]
        for i in digest.matches.indices {
            var m = digest.matches[i]
            if m.sport == "futbol", let model = StatModel.football(homeFor: m.goalsForHome, homeAgainst: m.goalsAgainstHome,
                                                                   awayFor: m.goalsForAway, awayAgainst: m.goalsAgainstAway) {
                m.model1X2 = [model.homeWin, model.draw, model.awayWin]
                m.modelTotals = model.totals
                m.modelScores = model.likelyScores.map { "\($0.home)-\($0.away) · \(Int(($0.p * 100).rounded())) %" }
                for k in m.markets.indices {
                    m.markets[k].model = StatModel.probability(of: m.markets[k].name, home: m.home, away: m.away, model: model)
                }
                if let market = m.pick?.market { m.pick?.model = StatModel.probability(of: market, home: m.home, away: m.away, model: model) }
                byName[key(m.match)] = model
            } else if m.sport == "tenis", let p = StatModel.tennis(rankA: m.rankHome, rankB: m.rankAway,
                                                                  surfaceA: m.surfaceHome, surfaceB: m.surfaceAway) {
                m.modelTennis = p
                for k in m.markets.indices { m.markets[k].model = tennisWinner(m.markets[k].name, home: m.home, away: m.away, p: p) }
                if let market = m.pick?.market { m.pick?.model = tennisWinner(market, home: m.home, away: m.away, p: p) }
                tennisByName[key(m.match)] = (m.home, m.away, p)
            }
            digest.matches[i] = m
        }
        for i in digest.picks.indices {
            let k = key(digest.picks[i].match)
            if let model = byName[k], let m = digest.matches.first(where: { key($0.match) == k }) {
                digest.picks[i].model = StatModel.probability(of: digest.picks[i].market, home: m.home, away: m.away, model: model)
            } else if let t = tennisByName[k] {
                digest.picks[i].model = tennisWinner(digest.picks[i].market, home: t.home, away: t.away, p: t.p)
            }
        }
    }

    private static func tennisWinner(_ market: String, home: String, away: String, p: Double) -> Double? {
        let m = market.lowercased()
        guard m.contains("gana") || m.contains("ganador") || m.contains("victoria") else { return nil }
        if m.contains("set") || m.contains("juego") || m.contains("hándicap") || m.contains("handicap") { return nil }
        let folded = StatModel.fold(market)
        if StatModel.team(home, in: folded, other: away) { return p }
        if StatModel.team(away, in: folded, other: home) { return 1 - p }
        return nil
    }

    static func key(_ match: String) -> String {
        match.lowercased().folding(options: .diacriticInsensitive, locale: Locale(identifier: "es"))
            .filter { $0.isLetter || $0.isNumber }
    }

    /// Text without links: "[UEFA](https://…)" keeps "UEFA", a bare address goes away.
    static func plain(_ text: String) -> String {
        var out = text.replacingOccurrences(of: #"\[([^\]]+)\]\([^)]*\)?"#, with: "$1", options: .regularExpression)
        out = out.replacingOccurrences(of: #"\(?https?://\S*"#, with: "", options: .regularExpression)
        out = out.replacingOccurrences(of: #"\s{2,}"#, with: " ", options: .regularExpression)
        return out.trimmingCharacters(in: .whitespaces)
    }

    /// "WWDLW" from whatever PARLEY wrote ("G-E-P", "W D L"…): W, D, L only, at most 10.
    static func form(_ text: String) -> String {
        let mapped = text.uppercased().compactMap { ch -> Character? in
            switch ch {
            case "W", "G", "V": return "W"
            case "D", "E": return "D"
            case "L", "P": return "L"
            default: return nil
            }
        }
        return String(mapped.suffix(10))
    }

    static func confidence(_ text: String) -> String {
        let t = text.lowercased()
        return t.hasPrefix("a") ? "alta" : t.hasPrefix("m") ? "media" : t.hasPrefix("b") ? "baja" : ""
    }

    private static func number(_ value: JSONValue) -> Double? {
        switch value {
        case .number(let n): return n.isFinite ? n : nil
        case .string(let s): return Double(s.replacingOccurrences(of: ",", with: ".").replacingOccurrences(of: "%", with: "")
                                            .trimmingCharacters(in: .whitespaces))
        default: return nil
        }
    }

    /// 0.58, "58%", 58 → 0.58; nil outside (0, 1].
    static func probability(_ value: JSONValue) -> Double? {
        guard var p = number(value) else { return nil }
        if p > 1 { p /= 100 }
        return p > 0 && p <= 1 ? p : nil
    }

    private static func odds(_ value: JSONValue) -> Double? {
        guard let o = number(value), o > 1, o < 1000 else { return nil }
        return o
    }

    private static func decode(_ text: String) -> JSONValue? {
        try? JSONDecoder().decode(JSONValue.self, from: Data(text.utf8))
    }

    private static func trimmedToLastBrace(_ text: String) -> String {
        guard let end = text.lastIndex(of: "}") else { return text }
        return String(text[...end])
    }

    /// An answer cut off mid-way (the turn's output ran out): kept up to its last complete value, then every open array and
    /// object is closed. Nil when nothing complete is left.
    static func repairTruncated(_ text: String) -> String? {
        var stack: [Character] = []
        var inString = false, escaped = false
        var lastGood: (end: String.Index, stack: [Character])?
        var i = text.startIndex
        while i < text.endIndex {
            let ch = text[i]
            if inString {
                if escaped { escaped = false } else if ch == "\\" { escaped = true } else if ch == "\"" { inString = false }
            } else {
                switch ch {
                case "\"": inString = true
                case "{", "[": stack.append(ch)
                case "}", "]":
                    guard !stack.isEmpty else { return nil }
                    stack.removeLast()
                    if stack.isEmpty { return String(text[...i]) }
                    lastGood = (text.index(after: i), stack)
                default: break
                }
            }
            i = text.index(after: i)
        }
        guard let good = lastGood else { return nil }
        var out = String(text[..<good.end])
        for open in good.stack.reversed() { out.append(open == "{" ? "}" : "]") }
        return out
    }

    // MARK: - What PARLEY says in its chat

    /// The summary that lands in PARLEY's chat with the page's button: the day in a few lines and the best picks.
    static func chatSummary(_ d: Digest) -> String {
        let football = d.matches.filter { $0.sport == "futbol" }.count
        let tennis = d.matches.count - football
        var out = "**Análisis del día listo** · \(d.matches.count) partidos (\(football) de fútbol, \(tennis) de tenis)"
        if !d.tips.isEmpty { out += " y \(d.tips.count) picks de Telegram revisados" }
        out += ".\n\n"
        if !d.summary.isEmpty { out += d.summary + "\n\n" }
        if !d.picks.isEmpty {
            out += "**Mejores picks**\n"
            for (n, p) in d.picks.prefix(5).enumerated() {
                let prob = p.prob.map { " · \(pct($0))" } ?? ""
                let price = p.odds.map { " @\(String(format: "%.2f", $0))" } ?? (StatModel.fairOdds(p.prob).map { " (juega desde @\(String(format: "%.2f", $0)))" } ?? "")
                let stake = p.stake.isEmpty ? "" : " · \(p.stake)"
                out += "\(n + 1). \(p.match): \(p.market)\(price)\(prob)\(stake)\n"
            }
        }
        let favor = d.tips.filter { $0.verdict == "a favor" }.count
        if !d.tips.isEmpty { out += "\nTelegram: \(favor) a favor, \(d.tips.count - favor) dudosos o en contra.\n" }
        out += "\nEl detalle, con gráficos y el modelo de MIKA, está en el resumen."
        return out
    }

    static func pct(_ p: Double) -> String { "\(Int((p * 100).rounded())) %" }

    // MARK: - The page

    static func escape(_ text: String) -> String {
        var out = ""
        for ch in text {
            switch ch {
            case "&": out += "&amp;"
            case "<": out += "&lt;"
            case ">": out += "&gt;"
            case "\"": out += "&quot;"
            case "'": out += "&#39;"
            default: out.append(ch)
            }
        }
        return out
    }

    /// A source as a small link (host only), or nothing when it is not an http(s) address.
    static func sourceLink(_ source: String) -> String {
        guard let url = ExternalLink.validated(source), let host = Picks.webHost(url.absoluteString) else { return "" }
        return "<a href=\"\(escape(url.absoluteString))\" target=\"_blank\" rel=\"noopener noreferrer\">\(escape(host))</a>"
    }

    private static func n1(_ v: Double) -> String { String(format: "%.1f", v) }
    private static func n2(_ v: Double) -> String { String(format: "%.2f", v) }
    private static func percent(_ p: Double?) -> String { p.map { "\(Int(($0 * 100).rounded()))%" } ?? "—" }
    private static func signed(_ edge: Double) -> String {
        let v = edge * 100
        return (v >= 0 ? "+" : "−") + String(format: "%.1f", abs(v)) + "%"
    }
    private static func edgeCell(_ edge: Double?) -> String {
        guard let edge else { return "<td class=\"dim\">—</td>" }
        return "<td class=\"\(edge >= 0 ? "pos" : "neg")\">\(signed(edge))</td>"
    }

    private static func conf(_ c: String) -> String {
        guard !c.isEmpty else { return "" }
        return "<span class=\"conf conf-\(c)\"><i></i><i></i><i></i>\(c)</span>"
    }

    /// Probability against the price: the bar is the probability, the tick the price's implied one.
    private static func minibar(_ p: Double?, odds: Double?, fill: String = "var(--accent)") -> String {
        var out = "<svg class=\"minibar\" viewBox=\"0 0 100 12\" preserveAspectRatio=\"none\" aria-hidden=\"true\"><rect x=\"0\" y=\"3\" width=\"100\" height=\"6\" fill=\"var(--track)\"/>"
        if let p { out += "<rect x=\"0\" y=\"3\" width=\"\(n1(p * 100))\" height=\"6\" fill=\"\(fill)\"/>" }
        if let imp = StatModel.implied(odds) { out += "<rect x=\"\(n1(imp * 100 - 0.6))\" y=\"0\" width=\"1.2\" height=\"12\" fill=\"var(--ink)\"/>" }
        return out + "</svg>"
    }

    private static func formChips(_ letters: String) -> String {
        guard !letters.isEmpty else { return "<span class=\"dim\">sin datos</span>" }
        let chips = letters.map { ch -> String in
            switch ch {
            case "W": return "<li class=\"w\">G</li>"
            case "D": return "<li class=\"d\">E</li>"
            default: return "<li class=\"l\">P</li>"
            }
        }
        return "<ol class=\"form\">" + chips.joined() + "</ol>"
    }

    private static func dayTitle(_ day: String) -> String {
        let parser = DateFormatter()
        parser.locale = Locale(identifier: "en_US_POSIX")
        parser.dateFormat = "yyyy-MM-dd"
        guard let date = parser.date(from: day) else { return day }
        let out = DateFormatter()
        out.locale = Locale(identifier: "es")
        out.dateFormat = "EEEE d 'de' MMMM 'de' yyyy"
        let text = out.string(from: date)
        return text.prefix(1).uppercased() + text.dropFirst()
    }

    /// "S/ 8" → 8, for the day's total.
    private static func money(_ stake: String) -> Double? {
        let digits = stake.filter { $0.isNumber || $0 == "." || $0 == "," }.replacingOccurrences(of: ",", with: ".")
        return Double(digits)
    }

    /// "vie 2 oct · 19:00" (the day of the match and its hour).
    static func when(date: String, time: String) -> String {
        let parser = DateFormatter()
        parser.locale = Locale(identifier: "en_US_POSIX")
        parser.dateFormat = "yyyy-MM-dd"
        guard let day = parser.date(from: date) else { return time }
        let out = DateFormatter()
        out.locale = Locale(identifier: "es")
        out.dateFormat = "EEE d MMM"
        let label = out.string(from: day).replacingOccurrences(of: ".", with: "")
        return time.isEmpty ? label : "\(label) · \(time)"
    }

    /// The leagues the filter offers: the ones with most matches, at most ten.
    private static func leagues(_ matches: [DigestMatch]) -> [String] {
        let counts = Dictionary(grouping: matches.filter { !$0.tournament.isEmpty }, by: \.tournament).mapValues(\.count)
        return counts.sorted { $0.value != $1.value ? $0.value > $1.value : $0.key < $1.key }.prefix(10).map(\.key)
    }

    /// The filter tokens of a match (`data-f`): its sport, "fuerte", "cuota" and its league's id.
    private static func tokens(_ m: DigestMatch, leagues: [String]) -> String {
        var out = [m.sport]
        if m.level == "fuerte" { out.append("fuerte") }
        if let i = leagues.firstIndex(of: m.tournament) { out.append("lg\(i)") }
        return out.joined(separator: " ")
    }

    /// The match a pick belongs to.
    private static func match(of pick: DigestPick, in matches: [DigestMatch]) -> DigestMatch? {
        matches.first { key($0.match) == key(pick.match) }
    }

    /// The Telegram tips that talk about this match (a word of a team's name in the tip).
    static func tips(about m: DigestMatch, in tips: [DigestTip]) -> [DigestTip] {
        tips.filter { t in
            let folded = StatModel.fold(t.pick)
            return StatModel.team(m.home, in: folded, other: m.away) || StatModel.team(m.away, in: folded, other: m.home)
        }
    }

    static func html(_ d: Digest) -> String {
        let football = d.matches.filter { $0.sport == "futbol" }
        let tennis = d.matches.filter { $0.sport == "tenis" }
        let strong = d.matches.filter { $0.level == "fuerte" }.count
        let edges = d.picks.compactMap { StatModel.edge(probability: $0.prob, odds: $0.odds) }
        let share = d.picks.compactMap { money($0.stake) }.reduce(0, +)
        let lgs = leagues(d.matches)

        // The filters are radio buttons at the top of the page, styled by CSS only (no scripts).
        var b = "<input type=\"checkbox\" id=\"theme\" aria-label=\"Cambiar tema claro u oscuro\">"
        for (id, on) in [("v-todo", true), ("v-picks", false), ("v-telegram", false), ("v-partidos", false)] {
            b += "<input type=\"radio\" name=\"vista\" id=\"\(id)\" class=\"flt\"\(on ? " checked" : "")>"
        }
        for (id, on) in [("f-all", true), ("f-futbol", false), ("f-tenis", false), ("f-fuerte", false)] {
            b += "<input type=\"radio\" name=\"liga\" id=\"\(id)\" class=\"flt\"\(on ? " checked" : "")>"
        }
        for i in lgs.indices { b += "<input type=\"radio\" name=\"liga\" id=\"f-lg\(i)\" class=\"flt\">" }
        b += "<input type=\"radio\" name=\"orden\" id=\"o-solidos\" class=\"flt\" checked><input type=\"radio\" name=\"orden\" id=\"o-valor\" class=\"flt\">"
        b += "<style>\(filterCSS(leagues: lgs.count))</style>"

        // Header
        b += "<header class=\"top\"><div class=\"wrap\"><div class=\"top-bar\"><span class=\"eyebrow\">PARLEY · análisis del día</span><label for=\"theme\" class=\"theme-btn\">Tema</label></div>"
        b += "<div class=\"top-grid\"><div><h1>\(escape(dayTitle(d.day)))</h1><p class=\"gen\">Generado a las \(escape(d.generated)), hora de Perú. Montos en % de tu banca.</p>"
        if !d.summary.isEmpty { b += "<p class=\"resumen\">\(escape(d.summary))</p>" }
        b += "</div><dl class=\"kpis\">"
        b += "<div class=\"kpi\"><dt>Partidos analizados</dt><dd>\(d.matches.count)<small>\(strong) a fondo</small></dd></div>"
        b += "<div class=\"kpi\"><dt>Fútbol</dt><dd>\(football.count)</dd></div>"
        b += "<div class=\"kpi\"><dt>Tenis</dt><dd>\(tennis.count)</dd></div>"
        b += "<div class=\"kpi\"><dt>Picks</dt><dd>\(d.picks.count)\(share > 0 ? "<small>\(Picks.fmt(share)) % banca</small>" : "")</dd></div>"
        if edges.isEmpty {
            b += "<div class=\"kpi\"><dt>Valor medio</dt><dd class=\"dim\">—</dd></div>"
        } else {
            let avg = edges.reduce(0, +) / Double(edges.count)
            b += "<div class=\"kpi\"><dt>Valor medio</dt><dd class=\"\(avg >= 0 ? "pos" : "neg")\">\(signed(avg))</dd></div>"
        }
        b += "<div class=\"kpi\"><dt>Telegram revisados</dt><dd>\(d.tips.count)<small>tips</small></dd></div></dl></div></div></header>"

        // Filter bar
        b += "<nav class=\"nav\" aria-label=\"Filtros\"><div class=\"wrap fbar\"><div class=\"fgroup\"><span class=\"flabel\">Ver</span>"
        b += "<label for=\"v-todo\">Todo</label><label for=\"v-picks\">Picks <span>\(d.picks.count)</span></label>"
        b += "<label for=\"v-telegram\">Telegram <span>\(d.tips.count)</span></label><label for=\"v-partidos\">Partidos <span>\(d.matches.count)</span></label></div>"
        b += "<div class=\"fgroup\"><span class=\"flabel\">Liga</span><label for=\"f-all\">Todas</label><label for=\"f-fuerte\">Fuertes <span>\(strong)</span></label>"
        b += "<label for=\"f-futbol\">Fútbol <span>\(football.count)</span></label><label for=\"f-tenis\">Tenis <span>\(tennis.count)</span></label>"
        for (i, name) in lgs.enumerated() { b += "<label for=\"f-lg\(i)\">\(escape(name))</label>" }
        b += "</div><div class=\"fgroup\"><span class=\"flabel\">Orden</span><label for=\"o-solidos\">Más sólidos</label><label for=\"o-valor\">Más valor</label></div></div></nav><main class=\"wrap\">"

        b += picksSection(d, leagues: lgs, share: share)
        if !d.combos.isEmpty { b += combosSection(d.combos) }
        b += tipsSection(d.tips)
        b += "<div id=\"partidos\">"
        b += matchesSection(id: "futbol", title: "Fútbol", list: football,
                            note: "Probabilidades 1X2 y de goles del modelo Poisson de MIKA, con los promedios que encontró PARLEY.", picks: d.picks, leagues: lgs)
        b += matchesSection(id: "tenis", title: "Tenis", list: tennis,
                            note: "Probabilidad de victoria del modelo de ranking y superficie de MIKA.", picks: d.picks, leagues: lgs)
        b += "</div>"
        if !d.warnings.isEmpty {
            b += "<section class=\"sec\" id=\"ojo\"><div class=\"sec-head\"><h2>Ojo</h2></div><div class=\"panel method\"><ul class=\"stats\">"
            b += d.warnings.map { "<li>\(escape($0))</li>" }.joined() + "</ul></div></section>"
        }
        b += methodSection(d.method)
        b += "</main><footer class=\"foot\"><div class=\"wrap\"><b>MIKA no apuesta: PARLEY propone y la decisión es tuya.</b> Juega con responsabilidad y solo lo que estés dispuesto a perder.</div></footer>"
        return page(title: "PARLEY · \(d.day)", body: b)
    }

    /// What the filter radios do: the view hides sections, the league keeps the cards that carry its token, the order
    /// swaps the two lists of picks, and the chosen chip lights up.
    private static func filterCSS(leagues: Int) -> String {
        var c = ".flt{position:absolute;opacity:0;width:1px;height:1px;pointer-events:none}"
        c += "html:has(#v-picks:checked) :is(#telegram,#partidos,#combinadas){display:none}"
        c += "html:has(#v-telegram:checked) :is(#picks,#partidos,#combinadas){display:none}"
        c += "html:has(#v-partidos:checked) :is(#picks,#telegram,#combinadas){display:none}"
        var ids: [(String, String)] = [("f-futbol", "futbol"), ("f-tenis", "tenis"), ("f-fuerte", "fuerte")]
        for i in 0..<leagues { ids.append(("f-lg\(i)", "lg\(i)")) }
        for (id, token) in ids { c += "html:has(#\(id):checked) [data-f]:not([data-f~=\"\(token)\"]){display:none}" }
        c += ".list-value{display:none}html:has(#o-valor:checked) .list-value{display:grid}html:has(#o-valor:checked) .list-solid{display:none}"
        let all = ["v-todo", "v-picks", "v-telegram", "v-partidos", "f-all", "f-futbol", "f-tenis", "f-fuerte", "o-solidos", "o-valor"]
            + (0..<leagues).map { "f-lg\($0)" }
        c += all.map { "html:has(#\($0):checked) label[for=\"\($0)\"]" }.joined(separator: ",")
            + "{background:var(--accent);border-color:var(--accent);color:#fff}"
        c += all.map { "html:has(#\($0):checked) label[for=\"\($0)\"] span" }.joined(separator: ",") + "{color:rgba(255,255,255,.8)}"
        return c
    }

    private static func picksSection(_ d: Digest, leagues: [String], share: Double) -> String {
        var b = "<section class=\"sec\" id=\"picks\"><div class=\"sec-head\"><h2>Picks del día</h2>"
        b += "<span class=\"sec-note\">Cada pick con su sustento: tendencias, modelo y por qué gana a los otros mercados del partido.\(share > 0 ? " Total: \(Picks.fmt(share)) % de tu banca." : "")</span></div>"
        guard !d.picks.isEmpty else { return b + "<p class=\"sec-note\">PARLEY no entregó la lista de picks.</p></section>" }
        let solid = Array(d.picks.enumerated())
        let value = solid.sorted {
            (StatModel.edge(probability: $0.element.prob, odds: $0.element.odds) ?? -1) > (StatModel.edge(probability: $1.element.prob, odds: $1.element.odds) ?? -1)
        }
        for (cls, list) in [("list-solid", solid), ("list-value", value)] {
            b += "<div class=\"scards \(cls)\">"
            for (rank, item) in list.enumerated() { b += scoutCard(item.element, rank: rank + 1, d: d, leagues: leagues) }
            b += "</div>"
        }
        return b + "</section>"
    }

    /// One pick, Scout-style: where and when, the market with its price and stake, the three probabilities as labelled
    /// bars, the trends with their hit rate, the reason, and the match's other markets for comparison.
    private static func scoutCard(_ p: DigestPick, rank: Int, d: Digest, leagues: [String]) -> String {
        let m = match(of: p, in: d.matches)
        var c = "<article class=\"panel scard\"\(m.map { " data-f=\"\(tokens($0, leagues: leagues))\"" } ?? "")>"
        c += "<div class=\"sc-top\"><span class=\"sc-rank\">\(rank)</span>"
        if let m {
            if !m.tournament.isEmpty { c += "<span class=\"sc-lg\">\(escape(m.tournament))</span>" }
            if m.level == "fuerte" { c += "<span class=\"sc-strong\">a fondo</span>" }
            c += "<time>\(escape(when(date: m.date, time: m.time)))</time>"
        }
        c += "</div><div class=\"sc-match\">\(escape(m.map { "\($0.home) vs \($0.away)" } ?? p.match))</div>"
        c += "<div class=\"sc-market\"><b>\(escape(p.market))</b>"
        if let odds = p.odds { c += "<span class=\"sc-odds\">@\(n2(odds))</span>" }
        else if let fair = StatModel.fairOdds(p.prob) { c += "<span class=\"sc-odds dim\">desde @\(n2(fair))</span>" }
        c += "</div><div class=\"sc-meta\">\(conf(p.confidence))"
        if !p.stake.isEmpty { c += "<span class=\"sc-stake\">\(escape(p.stake))</span>" }
        if let edge = StatModel.edge(probability: p.prob, odds: p.odds) {
            c += "<span class=\"sc-edge \(edge >= 0 ? "pos" : "neg")\">valor \(signed(edge))</span>"
        }
        c += "</div>"
        // The three probabilities, each on its own labelled bar.
        c += "<div class=\"sc-probs\">"
        let rows: [(String, Double?, String)] = [("PARLEY", p.prob, "var(--accent)"), ("Modelo MIKA", p.model, "var(--model)"),
                                                  ("La cuota implica", StatModel.implied(p.odds), "var(--implied)")]
        for (label, value, color) in rows where value != nil {
            c += "<div class=\"pr\"><span>\(label)</span><svg viewBox=\"0 0 100 8\" preserveAspectRatio=\"none\" aria-hidden=\"true\"><rect width=\"100\" height=\"8\" rx=\"2\" fill=\"var(--track)\"/><rect width=\"\(n1(value! * 100))\" height=\"8\" rx=\"2\" fill=\"\(color)\"/></svg><b>\(percent(value))</b></div>"
        }
        c += "</div>"
        // Why it lands: the trends with their hit rate.
        if let trends = m?.trends, !trends.isEmpty {
            c += "<h5>Por qué se cumple</h5><ul class=\"trends\">"
            for t in trends.prefix(4) {
                c += "<li><span>\(escape(t.text))</span>"
                if let hits = t.hits, let of = t.of, of > 0 {
                    let rate = Double(hits) / Double(of)
                    c += "<span class=\"hit \(rate >= 0.7 ? "hi" : rate >= 0.5 ? "mid" : "lo")\">\(hits)/\(of)</span>"
                }
                c += "</li>"
            }
            c += "</ul>"
        }
        if !p.reason.isEmpty { c += "<p class=\"sc-why\">\(escape(p.reason))</p>" }
        // Against the match's other markets: this one should stand out.
        if let markets = m?.markets, markets.count > 1 {
            c += "<h5>Frente a otros mercados</h5><ul class=\"vs-mk\">"
            let sorted = markets.sorted { (StatModel.edge(probability: $0.prob, odds: $0.odds) ?? ($0.prob ?? 0) - 1) > (StatModel.edge(probability: $1.prob, odds: $1.odds) ?? ($1.prob ?? 0) - 1) }
            for mk in sorted.prefix(5) {
                let mine = mk.name.lowercased() == p.market.lowercased()
                let edge = StatModel.edge(probability: mk.prob, odds: mk.odds)
                c += "<li\(mine ? " class=\"mine\"" : "")><span>\(escape(mk.name))</span>\(minibar(mk.prob, odds: mk.odds))<b>\(percent(mk.prob))</b>"
                c += edge.map { "<i class=\"\($0 >= 0 ? "pos" : "neg")\">\(signed($0))</i>" } ?? "<i class=\"dim\">\(mk.odds == nil ? "sin cuota" : "—")</i>"
                c += "</li>"
            }
            c += "</ul>"
        }
        if let m {
            let related = tips(about: m, in: d.tips)
            if !related.isEmpty {
                c += "<p class=\"sc-tg\">Telegram: " + related.prefix(3).map { "\(escape($0.channel)) · \(escape($0.verdict))" }.joined(separator: " · ") + "</p>"
            }
        }
        return c + "</article>"
    }

    private static func combosSection(_ combos: [DigestCombo]) -> String {
        var b = "<section class=\"sec\" id=\"combinadas\"><div class=\"sec-head\"><h2>Combinadas</h2><span class=\"sec-note\">Probabilidad conjunta estimada por PARLEY.</span></div><div class=\"combos\">"
        for (i, c) in combos.enumerated() {
            b += "<article class=\"panel combo\"><div class=\"combo-head\"><h3>Combinada \(i + 1) · \(c.legs.count) selecciones</h3></div><ul class=\"legs\">"
            for leg in c.legs {
                // "Equipo gana @1.67": the price after the @ goes to its own column.
                let parts = leg.components(separatedBy: " @")
                let price = parts.count > 1 ? "@" + parts.last! : ""
                let name = parts.count > 1 ? parts.dropLast().joined(separator: " @") : leg
                b += "<li class=\"leg\"><div class=\"leg-m\"><b>\(escape(name))</b></div><span></span><span>\(escape(price))</span></li>"
            }
            b += "</ul><dl class=\"combo-nums\"><div><dt>Prob. conjunta</dt><dd>\(percent(c.prob))</dd></div>"
            b += "<div><dt>Cuota total</dt><dd>\(c.odds.map(n2) ?? "—")</dd></div>"
            if let edge = StatModel.edge(probability: c.prob, odds: c.odds) {
                b += "<div><dt>Edge</dt><dd class=\"\(edge >= 0 ? "pos" : "neg")\">\(signed(edge))</dd></div>"
            } else {
                b += "<div><dt>Edge</dt><dd class=\"dim\">—</dd></div>"
            }
            b += "<div><dt>Monto</dt><dd>\(c.stake.isEmpty ? "—" : escape(c.stake))</dd></div></dl>"
            if !c.reason.isEmpty { b += "<p class=\"combo-why\">\(escape(c.reason))</p>" }
            b += "</article>"
        }
        return b + "</div></section>"
    }

    private static func tipsSection(_ tips: [DigestTip]) -> String {
        let channels = Set(tips.map(\.channel)).count
        var b = "<section class=\"sec\" id=\"telegram\"><div class=\"sec-head\"><h2>Picks de Telegram</h2>"
        b += "<span class=\"sec-note\">\(tips.count) tips de \(channels) canal\(channels == 1 ? "" : "es"), revisados con estadística.</span></div>"
        guard !tips.isEmpty else { return b + "<div class=\"panel method\"><p class=\"note\">Hoy tus canales no publicaron picks.</p></div></section>" }
        b += "<div class=\"panel\">"
        for t in tips {
            let verdictClass = t.verdict == "a favor" ? "v-favor" : t.verdict == "en contra" ? "v-contra" : "v-dudoso"
            let fill = t.verdict == "a favor" ? "var(--pos)" : t.verdict == "en contra" ? "var(--neg)" : "var(--warn)"
            b += "<article class=\"tip\"><div class=\"tip-src\"><b>\(escape(t.channel.isEmpty ? "Canal" : t.channel))</b>"
            if !t.author.isEmpty { b += "<span>\(escape(t.author))</span>" }
            if !t.time.isEmpty { b += "<time>\(escape(t.time))</time>" }
            b += "</div><div class=\"tip-main\"><div class=\"tip-pick\">\(escape(t.pick))\(t.odds.map { "<span class=\"odds\">@\(n2($0))</span>" } ?? "")</div>"
            if !t.reason.isEmpty { b += "<p class=\"tip-why\">\(escape(t.reason))</p>" }
            b += "</div><span class=\"ev\">\(escape(t.evidence.isEmpty ? "texto" : t.evidence))</span>"
            let implied = StatModel.implied(t.odds).map { "impl. \(n1($0 * 100))%" } ?? "sin cuota"
            b += "<div class=\"tip-prob\"><div class=\"tip-nums\"><b>\(percent(t.prob))</b>\(implied)</div>\(minibar(t.prob, odds: t.odds, fill: fill))</div>"
            b += "<span class=\"verdict \(verdictClass)\">\(escape(t.verdict))</span></article>"
        }
        return b + "</div></section>"
    }

    private static func matchesSection(id: String, title: String, list: [DigestMatch], note: String, picks: [DigestPick],
                                       leagues: [String]) -> String {
        var b = "<section class=\"sec\" id=\"\(id)\"><div class=\"sec-head\"><h2>\(title)</h2><span class=\"sec-note\">\(list.count) partidos. \(escape(note))</span></div>"
        guard !list.isEmpty else { return b + "<div class=\"panel method\"><p class=\"note\">Sin partidos en el análisis.</p></div></section>" }
        let groups = Dictionary(grouping: list, by: { $0.tournament.isEmpty ? "Otros" : $0.tournament })
        let ordered = groups.keys.sorted { (groups[$0]!.map(\.time).min() ?? "") < (groups[$1]!.map(\.time).min() ?? "") }
        var index = 0
        for name in ordered {
            let group = groups[name]!.sorted(by: { ($0.date, $0.time) < ($1.date, $1.time) })
            b += "<div class=\"tgroup\" data-f=\"\(group.map { tokens($0, leagues: leagues) }.joined(separator: " "))\">"
            b += "<h3 class=\"tourn\">\(escape(name))</h3>"
            for m in group {
                index += 1
                b += matchCard(m, id: "\(id)-\(index)", picks: picks, filter: tokens(m, leagues: leagues))
            }
            b += "</div>"
        }
        return b + "</section>"
    }

    private static func matchCard(_ m: DigestMatch, id: String, picks: [DigestPick], filter: String) -> String {
        let football = m.sport == "futbol"
        var b = "<article class=\"panel match \(football ? "football" : "tennis")\" id=\"\(id)\" data-f=\"\(filter)\"><header class=\"match-head\">"
        b += "<div class=\"match-meta\"><time>\(escape(when(date: m.date, time: m.time)))</time><span>\(escape(m.tournament))</span>\(m.level == "fuerte" ? "<span class=\"sc-strong\">a fondo</span>" : "")</div>"
        b += "<h3 class=\"match-title\"><span>\(escape(m.home.isEmpty ? m.match : m.home))</span>"
        if !m.away.isEmpty { b += "<span class=\"vs\">vs</span><span>\(escape(m.away))</span>" }
        b += "</h3>"
        b += m.odds.isEmpty ? "<p class=\"odds-line none\">sin cuota en las tablas de Betano</p>" : "<p class=\"odds-line\">\(escape(m.odds))</p>"
        b += "</header><div class=\"match-body\">"

        // Facts
        b += "<section class=\"mcol facts\"><h4>Forma, últimos partidos</h4>"
        b += "<div class=\"form-row\"><span>\(escape(m.home))</span>\(formChips(m.formHome))</div>"
        b += "<div class=\"form-row\"><span>\(escape(m.away))</span>\(formChips(m.formAway))</div>"
        var facts = m.stats
        if !m.h2h.isEmpty { facts.append("Cara a cara: \(m.h2h)") }
        if !facts.isEmpty { b += "<h4>Datos clave</h4><ul class=\"stats\">" + facts.map { "<li>\(escape($0))</li>" }.joined() + "</ul>" }
        if !m.trends.isEmpty {
            b += "<h4>Tendencias</h4><ul class=\"trends\">"
            for t in m.trends.prefix(5) {
                b += "<li><span>\(escape(t.text))</span>"
                if let hits = t.hits, let of = t.of, of > 0 {
                    let rate = Double(hits) / Double(of)
                    b += "<span class=\"hit \(rate >= 0.7 ? "hi" : rate >= 0.5 ? "mid" : "lo")\">\(hits)/\(of)</span>"
                }
                b += "</li>"
            }
            b += "</ul>"
        }
        b += "<h4>\(football ? "Bajas" : "Físico")</h4><p class=\"note\">\(escape(m.absences.isEmpty ? "Sin información de bajas." : m.absences))</p>"
        if !m.lineups.isEmpty {
            let confirmed = m.lineups.lowercased().contains("confirmad") && !m.lineups.lowercased().hasPrefix("probable")
            b += "<h4>\(football ? "Alineaciones" : "Orden de juego") <span class=\"lineup \(confirmed ? "lu-ok" : "lu-prob")\">\(confirmed ? "confirmadas" : "probables")</span></h4>"
            b += "<p class=\"note\">\(escape(m.lineups))</p>"
        }
        b += "</section>"

        // MIKA's model
        b += "<section class=\"mcol model\">"
        if football, let x = m.model1X2 {
            let h = x[0] * 100, d = x[1] * 100, a = x[2] * 100
            b += "<h4>1X2, modelo Poisson MIKA</h4>"
            b += "<svg class=\"split\" viewBox=\"0 0 100 10\" preserveAspectRatio=\"none\" role=\"img\" aria-label=\"1X2 del modelo\"><rect x=\"0\" y=\"0\" width=\"\(n1(h))\" height=\"10\" fill=\"var(--accent)\"/><rect x=\"\(n1(h))\" y=\"0\" width=\"\(n1(d))\" height=\"10\" fill=\"var(--draw)\"/><rect x=\"\(n1(h + d))\" y=\"0\" width=\"\(n1(a))\" height=\"10\" fill=\"var(--away)\"/></svg>"
            b += "<div class=\"split-labels\"><span>\(escape(short(m.home))) <b>\(Int(h.rounded()))%</b></span><span>Empate <b>\(Int(d.rounded()))%</b></span><span>\(escape(short(m.away))) <b>\(Int(a.rounded()))%</b></span></div>"
            if let totals = m.modelTotals {
                b += "<h4>Goles totales, distribución (%)</h4>" + goalsChart(totals)
                let over = 1 - totals[0] - totals[1] - totals[2]
                var caption = "Más de 2.5 (azul): \(n1(over * 100))%."
                if let hf = m.goalsForHome, let ha = m.goalsAgainstHome, let af = m.goalsForAway, let aa = m.goalsAgainstAway {
                    caption = "Goles esperados \(n2((hf + aa) / 2)) + \(n2((af + ha) / 2)). " + caption
                }
                if let scores = m.modelScores, !scores.isEmpty { caption += " Marcadores más probables: \(scores.joined(separator: ", "))." }
                b += "<p class=\"caption\">\(escape(caption))</p>"
            }
        } else if !football, let p = m.modelTennis {
            b += "<h4>Victoria, modelo ranking + superficie</h4>"
            b += "<svg class=\"split\" viewBox=\"0 0 100 10\" preserveAspectRatio=\"none\" role=\"img\" aria-label=\"Probabilidad de victoria\"><rect x=\"0\" y=\"0\" width=\"\(n1(p * 100))\" height=\"10\" fill=\"var(--accent)\"/><rect x=\"\(n1(p * 100))\" y=\"0\" width=\"\(n1(100 - p * 100))\" height=\"10\" fill=\"var(--away)\"/></svg>"
            b += "<div class=\"split-labels\"><span>\(escape(short(m.home))) <b>\(Int((p * 100).rounded()))%</b></span><span>\(escape(short(m.away))) <b>\(Int(((1 - p) * 100).rounded()))%</b></span></div>"
            b += duelChart(m)
        } else {
            b += "<h4>Modelo MIKA</h4><p class=\"note\">PARLEY no encontró los promedios que necesita el modelo; las probabilidades son solo las suyas.</p>"
        }
        b += "</section>"

        // Markets
        b += "<section class=\"mcol markets\"><h4>Mercados</h4>"
        if !m.markets.isEmpty {
            b += "<div class=\"table-wrap\"><table class=\"mk\"><thead><tr><th>Mercado</th><th>PARLEY</th><th>Modelo</th><th>Cuota</th><th>Justa</th><th>Edge</th><th>Prob. vs impl.</th></tr></thead><tbody>"
            let pickName = m.pick?.market.lowercased()
            for mk in m.markets {
                let isPick = pickName == mk.name.lowercased()
                b += "<tr\(isPick ? " class=\"is-pick\"" : "")><td>\(escape(mk.name))\(mk.note.isEmpty ? "" : "<br><span class=\"dim\">\(escape(mk.note))</span>")</td>"
                b += "<td class=\"c-par\">\(percent(mk.prob))</td><td class=\"c-mod\">\(percent(mk.model))</td>"
                b += mk.odds.map { "<td>\(n2($0))</td>" } ?? "<td class=\"dim\">—</td>"
                b += "<td>\(StatModel.fairOdds(mk.prob).map(n2) ?? "—")</td>"
                b += edgeCell(StatModel.edge(probability: mk.prob, odds: mk.odds))
                b += "<td class=\"c-bar\">\(minibar(mk.prob, odds: mk.odds))</td></tr>"
            }
            b += "</tbody></table></div>"
        }
        if let pick = m.pick {
            let rank = picks.firstIndex { key($0.match) == key(m.match) && $0.market.lowercased() == pick.market.lowercased() }
            let title = (rank.map { "Pick \($0 + 1): " } ?? "Pick: ") + pick.market + (pick.odds.map { " @\(n2($0))" } ?? "")
            b += "<div class=\"match-pick\"><div class=\"mp-top\"><b>\(escape(title))</b>"
            if pick.odds == nil, let fair = StatModel.fairOdds(pick.prob) { b += "<span class=\"dim\">sin cuota, juega desde @\(n2(fair))</span>" }
            b += "<span class=\"dim\">PARLEY \(percent(pick.prob))\(pick.model.map { " · modelo \(percent($0))" } ?? "")</span>\(conf(pick.confidence))</div>"
            if !pick.reason.isEmpty { b += "<p>\(escape(pick.reason))</p>" }
            b += "</div>"
        }
        b += "</section></div>"
        let links = m.sources.map(sourceLink).filter { !$0.isEmpty }
        if !links.isEmpty { b += "<footer class=\"match-src\">Fuentes: " + links.joined() + "</footer>" }
        return b + "</article>"
    }

    /// "Sporting Cristal" stays, longer names keep their last word.
    private static func short(_ name: String) -> String {
        name.count <= 14 ? name : String(name.split(separator: " ").last ?? Substring(name))
    }

    /// P(total goals = 0…5, 6+): grey up to two goals, accent from three.
    private static func goalsChart(_ totals: [Double]) -> String {
        var s = "<svg class=\"chart goals\" viewBox=\"0 0 210 92\" role=\"img\" aria-label=\"Distribución de goles totales\">"
        s += "<line x1=\"0\" x2=\"210\" y1=\"76.5\" y2=\"76.5\" stroke=\"var(--line-strong)\" stroke-width=\"1\"/>"
        s += "<line x1=\"90\" x2=\"90\" y1=\"4\" y2=\"78\" stroke=\"var(--muted)\" stroke-width=\"1\" stroke-dasharray=\"2 2\"/>"
        var bars = "<g>", values = "<g font-size=\"8\" text-anchor=\"middle\" fill=\"var(--ink-2)\">", labels = "<g font-size=\"8.5\" text-anchor=\"middle\" fill=\"var(--muted)\">"
        for k in 0..<min(totals.count, 7) {
            let p = totals[k] * 100
            let h = min(2.4 * p, 72)
            let x = Double(30 * k)
            bars += "<rect x=\"\(n1(x + 4))\" y=\"\(n1(76 - h))\" width=\"22\" height=\"\(n1(h))\" rx=\"1.5\" fill=\"var(\(k <= 2 ? "--draw" : "--accent"))\"/>"
            values += "<text x=\"\(n1(x + 15))\" y=\"\(n1(76 - h - 3))\">\(n1(p))</text>"
            labels += "<text x=\"\(n1(x + 15))\" y=\"88\">\(k == 6 ? "6+" : "\(k)")</text>"
        }
        return s + bars + "</g>" + values + "</g>" + labels + "</g></svg>"
    }

    /// Ranking and surface win rate, side by side (the better ranking gets the longest bar).
    private static func duelChart(_ m: DigestMatch) -> String {
        var rows: [(label: String, a: Double, b: Double, ta: String, tb: String)] = []
        if let ra = m.rankHome, let rb = m.rankAway, ra >= 1, rb >= 1 {
            let best = min(ra, rb)
            rows.append(("Ranking", 100 * best / ra, 100 * best / rb, "#\(Int(ra))", "#\(Int(rb))"))
        }
        if let sa = m.surfaceHome, let sb = m.surfaceAway {
            let a = sa > 1 ? sa : sa * 100, b = sb > 1 ? sb : sb * 100
            rows.append(("Victorias en \(m.surface.isEmpty ? "la superficie" : m.surface)", a, b, "\(Int(a.rounded()))%", "\(Int(b.rounded()))%"))
        }
        guard !rows.isEmpty else { return "" }
        let height = 24 * rows.count
        var s = "<h4>Comparativa</h4><div class=\"duel\"><div class=\"duel-names\"><span>\(escape(short(m.home)))</span><span>\(escape(short(m.away)))</span></div>"
        s += "<svg class=\"chart\" viewBox=\"0 0 220 \(height)\" role=\"img\" aria-label=\"Comparativa\">"
        for (r, row) in rows.enumerated() {
            let y = Double(24 * r)
            let la = 0.8 * min(max(row.a, 0), 100), lb = 0.8 * min(max(row.b, 0), 100)
            s += "<text x=\"110\" y=\"\(n1(y + 8))\" font-size=\"8\" fill=\"var(--muted)\" text-anchor=\"middle\">\(escape(row.label))</text>"
            s += "<rect x=\"\(n1(109 - la))\" y=\"\(n1(y + 11))\" width=\"\(n1(la))\" height=\"7\" rx=\"1\" fill=\"var(--accent)\"/>"
            s += "<rect x=\"111\" y=\"\(n1(y + 11))\" width=\"\(n1(lb))\" height=\"7\" rx=\"1\" fill=\"var(--away)\"/>"
            s += "<text x=\"\(n1(106 - la))\" y=\"\(n1(y + 17.5))\" font-size=\"8.5\" font-weight=\"600\" fill=\"var(--ink)\" text-anchor=\"end\">\(escape(row.ta))</text>"
            s += "<text x=\"\(n1(114 + lb))\" y=\"\(n1(y + 17.5))\" font-size=\"8.5\" font-weight=\"600\" fill=\"var(--ink)\">\(escape(row.tb))</text>"
        }
        return s + "</svg></div>"
    }

    private static func methodSection(_ method: String) -> String {
        var b = "<section class=\"sec\" id=\"metodo\"><div class=\"sec-head\"><h2>Método</h2></div><div class=\"panel method\">"
        b += "<div><h3>Fútbol: Poisson</h3><p>MIKA toma los goles a favor y en contra por partido que encontró PARLEY (el local en casa, el visitante fuera) y estima los goles esperados de cada lado. Dos distribuciones de Poisson independientes dan el 1X2, los totales, ambos marcan y los marcadores más probables.</p></div>"
        b += "<div><h3>Tenis: ranking y superficie</h3><p>Una logística sobre la diferencia de ranking (en escala logarítmica) se promedia con el log5 de los porcentajes de victoria de cada jugador en la superficie.</p></div>"
        b += "<div><h3>Cómo leer los números</h3><p>\(method.isEmpty ? "" : escape(method) + " ")PARLEY ajusta con contexto (bajas, alineaciones, cansancio, motivación); el modelo de MIKA es la referencia numérica. Cuota justa = 1/probabilidad; edge = probabilidad × cuota − 1. Son estimaciones, no certezas.</p></div>"
        return b + "</div></section>"
    }

    /// The page when PARLEY's answer was not the JSON asked for: its text, escaped, so nothing is lost.
    static func fallbackHTML(day: String, generated: String, text: String) -> String {
        let paragraphs = text.components(separatedBy: "\n").map { escape($0) }.map { $0.isEmpty ? "" : "<p class=\"note\">\($0)</p>" }.joined()
        var b = "<input type=\"checkbox\" id=\"theme\" aria-label=\"Cambiar tema claro u oscuro\">"
        b += "<header class=\"top\"><div class=\"wrap\"><div class=\"top-bar\"><span class=\"eyebrow\">PARLEY · análisis del día</span><label for=\"theme\" class=\"theme-btn\">Tema</label></div>"
        b += "<h1>\(escape(dayTitle(day)))</h1><p class=\"gen\">Generado a las \(escape(generated)). PARLEY no entregó la tabla en el formato esperado: esta es su respuesta tal cual.</p></div></header>"
        b += "<main class=\"wrap\"><section class=\"sec\"><div class=\"panel method\">\(paragraphs)</div></section></main>"
        return page(title: "PARLEY · \(day)", body: b)
    }

    private static func page(title: String, body: String) -> String {
        "<!doctype html><html lang=\"es\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">"
            + "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; img-src data:\">"
            + "<meta name=\"color-scheme\" content=\"light dark\"><title>\(escape(title))</title><style>\(css)</style></head><body>\(body)</body></html>"
    }

    /// The page's style (designed for this report: tokens for light and dark, a CSS-only theme switch, no scripts).
    private static let css = #"""
/* ---------- tokens ---------- */
:root{
  color-scheme: light;
  --font: -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI Variable Text", "Segoe UI", system-ui, Roboto, "Helvetica Neue", Arial, sans-serif;
  --bg:#F2F4F6; --surface:#FFFFFF; --surface-2:#F7F8FA;
  --ink:#121A23; --ink-2:#3A4652; --muted:#67737F;
  --line:#E1E5EA; --line-strong:#C9D0D8; --track:#E8ECF0;
  --accent:#2350C8; --accent-soft:#E8EEFC;
  --model:#7A52C7; --implied:#A3ADB8; --away:#C8742A; --draw:#BAC2CB;
  --pos:#1C8A4E; --pos-soft:#E2F3E9;
  --warn:#9E6406; --warn-soft:#FAEFD8;
  --neg:#C23B2A; --neg-soft:#FBE6E2;
}
html:has(#theme:checked){
  color-scheme: dark;
  --bg:#0F141A; --surface:#161C24; --surface-2:#1B232D;
  --ink:#E7ECF1; --ink-2:#BAC4CE; --muted:#8693A0;
  --line:#252E39; --line-strong:#344050; --track:#232C37;
  --accent:#6E97FF; --accent-soft:#1B2848;
  --model:#B194F2; --implied:#5E6B79; --away:#E59A55; --draw:#4A5664;
  --pos:#45C483; --pos-soft:#14301F;
  --warn:#E6B04A; --warn-soft:#352812;
  --neg:#F07862; --neg-soft:#3A1C17;
}
@media (prefers-color-scheme: dark){
  :root{
    color-scheme: dark;
    --bg:#0F141A; --surface:#161C24; --surface-2:#1B232D;
    --ink:#E7ECF1; --ink-2:#BAC4CE; --muted:#8693A0;
    --line:#252E39; --line-strong:#344050; --track:#232C37;
    --accent:#6E97FF; --accent-soft:#1B2848;
    --model:#B194F2; --implied:#5E6B79; --away:#E59A55; --draw:#4A5664;
    --pos:#45C483; --pos-soft:#14301F;
    --warn:#E6B04A; --warn-soft:#352812;
    --neg:#F07862; --neg-soft:#3A1C17;
  }
  html:has(#theme:checked){
    color-scheme: light;
    --bg:#F2F4F6; --surface:#FFFFFF; --surface-2:#F7F8FA;
    --ink:#121A23; --ink-2:#3A4652; --muted:#67737F;
    --line:#E1E5EA; --line-strong:#C9D0D8; --track:#E8ECF0;
    --accent:#2350C8; --accent-soft:#E8EEFC;
    --model:#7A52C7; --implied:#A3ADB8; --away:#C8742A; --draw:#BAC2CB;
    --pos:#1C8A4E; --pos-soft:#E2F3E9;
    --warn:#9E6406; --warn-soft:#FAEFD8;
    --neg:#C23B2A; --neg-soft:#FBE6E2;
  }
}

/* ---------- base ---------- */
*,*::before,*::after{box-sizing:border-box}
html{-webkit-text-size-adjust:100%}
@media (prefers-reduced-motion: no-preference){html{scroll-behavior:smooth}}
body{margin:0;background:var(--bg);color:var(--ink);font:13px/1.45 var(--font);font-variant-numeric:tabular-nums;-webkit-font-smoothing:antialiased;overflow-x:hidden}
h1,h2,h3,h4,p,ul,ol,dl,dd,figure{margin:0}
ul,ol{padding:0;list-style:none}
a{color:var(--accent);text-decoration:none}
a:hover{text-decoration:underline}
a:focus-visible,label:focus-visible{outline:2px solid var(--accent);outline-offset:2px;border-radius:3px}
.wrap{max-width:1240px;margin:0 auto;padding:0 16px}
@media (min-width:768px){.wrap{padding:0 24px}}
.num{font-variant-numeric:tabular-nums}
.pos{color:var(--pos)} .neg{color:var(--neg)} .dim{color:var(--muted)}

/* theme toggle (hidden checkbox + label) */
#theme{position:absolute;opacity:0;width:1px;height:1px;pointer-events:none}
.theme-btn{display:inline-flex;align-items:center;gap:6px;height:26px;padding:0 10px;border:1px solid var(--line-strong);border-radius:999px;color:var(--ink-2);font-size:11px;cursor:pointer;user-select:none;white-space:nowrap;background:var(--surface)}
.theme-btn::before{content:"";width:11px;height:11px;border-radius:50%;border:1.5px solid currentColor;background:linear-gradient(90deg,currentColor 50%,transparent 50%)}
.theme-btn:hover{border-color:var(--muted)}
html:has(#theme:focus-visible) .theme-btn{outline:2px solid var(--accent);outline-offset:2px}

/* ---------- header ---------- */
.top{padding:20px 0 18px;border-bottom:1px solid var(--line);background:var(--surface)}
.top-bar{display:flex;justify-content:space-between;align-items:center;gap:12px;margin-bottom:14px}
.eyebrow{font-size:11px;font-weight:600;color:var(--accent);letter-spacing:.02em}
.top-grid{display:grid;gap:18px}
@media (min-width:960px){.top-grid{grid-template-columns:minmax(0,1fr) 440px;gap:32px;align-items:start}}
h1{font-size:24px;line-height:1.15;font-weight:700;letter-spacing:-.015em}
.gen{margin-top:4px;font-size:11px;color:var(--muted)}
.resumen{margin-top:12px;max-width:68ch;color:var(--ink-2);font-size:13.5px;line-height:1.55}
.resumen b{color:var(--ink);font-weight:600}
.kpis{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));border:1px solid var(--line);border-radius:10px;overflow:hidden;background:var(--surface-2)}
.kpi{display:flex;flex-direction:column;justify-content:space-between;gap:2px;padding:9px 12px 10px;border-right:1px solid var(--line);border-bottom:1px solid var(--line)}
.kpi:nth-child(3n){border-right:0}
.kpi:nth-last-child(-n+3){border-bottom:0}
.kpi dt{font-size:10.5px;color:var(--muted)}
.kpi dd{font-size:20px;font-weight:650;letter-spacing:-.01em;line-height:1.2}
.kpi dd small{font-size:10.5px;font-weight:500;color:var(--muted);margin-left:3px}

/* ---------- sticky nav ---------- */
.nav{position:sticky;top:0;z-index:10;background:color-mix(in srgb,var(--bg) 88%,transparent);backdrop-filter:saturate(1.4) blur(10px);-webkit-backdrop-filter:saturate(1.4) blur(10px);border-bottom:1px solid var(--line)}
.nav ul{display:flex;gap:6px;padding:8px 0;overflow-x:auto;scrollbar-width:none}
.nav ul::-webkit-scrollbar{display:none}
.nav a{display:inline-flex;align-items:center;gap:5px;height:26px;padding:0 11px;border-radius:999px;border:1px solid var(--line-strong);background:var(--surface);color:var(--ink-2);font-size:11.5px;white-space:nowrap;text-decoration:none}
.nav a:hover{border-color:var(--accent);color:var(--accent)}
.nav a span{color:var(--muted);font-size:10.5px}

/* ---------- sections ---------- */
.sec{padding:26px 0 6px;scroll-margin-top:48px}
.sec-head{display:flex;flex-wrap:wrap;align-items:baseline;justify-content:space-between;gap:4px 16px;margin-bottom:12px}
h2{font-size:16px;font-weight:700;letter-spacing:-.01em}
.sec-note{font-size:11px;color:var(--muted)}
.panel{background:var(--surface);border:1px solid var(--line);border-radius:10px}
h4{font-size:11px;font-weight:600;color:var(--muted);margin:14px 0 6px;display:flex;align-items:center;gap:6px}
h4:first-child{margin-top:0}

/* badges */
.conf{display:inline-flex;align-items:center;gap:2px;font-size:10.5px;color:var(--ink-2);white-space:nowrap}
.conf i{display:block;width:4px;height:10px;border-radius:1px;background:var(--track)}
.conf i:last-of-type{margin-right:4px}
.conf-alta i,.conf-media i:nth-child(-n+2),.conf-baja i:first-child{background:var(--accent)}
.verdict{display:inline-block;padding:2px 7px;border-radius:4px;font-size:10.5px;font-weight:600;white-space:nowrap}
.v-favor{background:var(--pos-soft);color:var(--pos)}
.v-dudoso{background:var(--warn-soft);color:var(--warn)}
.v-contra{background:var(--neg-soft);color:var(--neg)}
.ev{display:inline-block;padding:1px 6px;border:1px solid var(--line-strong);border-radius:4px;font-size:10px;color:var(--ink-2);white-space:nowrap}
.lineup{font-weight:600;font-size:10px;padding:1px 6px;border-radius:4px}
.lu-ok{background:var(--pos-soft);color:var(--pos)}
.lu-prob{background:var(--warn-soft);color:var(--warn)}

/* ---------- picks ---------- */
.picks-grid{display:grid;gap:14px}
@media (min-width:1100px){.picks-grid{grid-template-columns:minmax(0,1fr) 380px;align-items:start}}
.pick{display:grid;grid-template-columns:22px minmax(0,1fr);gap:8px 10px;padding:12px 14px;border-bottom:1px solid var(--line)}
.pick:last-child{border-bottom:0}
.pick-rank{font-size:15px;font-weight:700;color:var(--muted);line-height:1.2}
.pick-top{display:flex;justify-content:space-between;gap:8px;align-items:center}
.pick-match{font-size:11px;color:var(--muted);overflow-wrap:anywhere}
.pick-market{font-size:14px;font-weight:600;margin-top:1px}
.pick-why{margin-top:3px;font-size:12px;color:var(--ink-2);max-width:72ch}
.pick-nums{grid-column:2;display:grid;grid-template-columns:repeat(5,minmax(0,1fr));gap:6px}
.pick-nums div{min-width:0}
.pick-nums dt{font-size:10px;color:var(--muted)}
.pick-nums dd{font-size:13px;font-weight:600}
.pick-nums dd small{display:block;font-size:10px;font-weight:500;color:var(--muted)}
.pick-nums .k-parley dd{color:var(--accent)}
.pick-nums .k-model dd{color:var(--model)}
@media (min-width:760px){
  .pick{grid-template-columns:22px minmax(0,1fr) 360px}
  .pick-nums{grid-column:3;grid-row:1;align-self:center}
}
.chart-panel{padding:14px 14px 12px}
.chart-panel h3{font-size:13px;font-weight:600}
.chart-panel p{font-size:11px;color:var(--muted);margin-top:2px}
.chart{display:block;width:100%;height:auto;overflow:visible}
.chart text{font-family:var(--font);font-variant-numeric:tabular-nums}
.gapchart{margin-top:10px}
.legend{display:flex;flex-wrap:wrap;gap:4px 12px;margin-top:8px;font-size:10.5px;color:var(--muted)}
.legend span{display:inline-flex;align-items:center;gap:5px}
.legend i{width:10px;height:7px;border-radius:1px;display:inline-block}
.lg-imp{background:var(--implied)} .lg-par{background:var(--accent)} .lg-gap{background:var(--pos);opacity:.35} .lg-mod{background:var(--model)}
.legend .lg-dot{width:8px;height:8px;border-radius:50%}
.legend .lg-dot.lg-imp{background:var(--surface);border:2px solid var(--implied)}

/* ---------- combinadas ---------- */
.combos{display:grid;gap:14px}
@media (min-width:900px){.combos{grid-template-columns:repeat(2,minmax(0,1fr))}}
.combo{padding:14px}
.combo-head{display:flex;justify-content:space-between;align-items:baseline;gap:8px}
.combo-head h3{font-size:14px;font-weight:600}
.legs{margin-top:10px;border-top:1px solid var(--line)}
.leg{display:grid;grid-template-columns:minmax(0,1fr) auto auto;gap:4px 12px;padding:7px 0;border-bottom:1px solid var(--line);font-size:12px;align-items:baseline}
.leg-m{min-width:0}
.leg-m b{font-weight:600;display:block}
.leg-m span{font-size:10.5px;color:var(--muted)}
.leg-p{color:var(--accent);font-weight:600}
.combo-nums{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:6px;margin-top:10px}
.combo-nums dt{font-size:10px;color:var(--muted)}
.combo-nums dd{font-size:15px;font-weight:650}
.combo-why{margin-top:8px;font-size:12px;color:var(--ink-2)}

/* ---------- telegram ---------- */
.tip{display:grid;grid-template-columns:minmax(0,1fr) auto;grid-template-areas:"src verdict" "main main" "prob ev";gap:6px 12px;padding:12px 14px;border-bottom:1px solid var(--line);align-items:center}
.tip:last-child{border-bottom:0}
.tip-src{grid-area:src;font-size:11px;color:var(--muted);display:flex;flex-wrap:wrap;gap:0 8px;min-width:0}
.tip-src b{color:var(--ink);font-weight:600}
.tip-src span{overflow-wrap:anywhere}
.tip-main{grid-area:main;min-width:0}
.tip-pick{font-size:13px;font-weight:600}
.tip-pick .odds{font-weight:500;color:var(--muted);margin-left:4px;white-space:nowrap}
.tip-why{font-size:12px;color:var(--ink-2);margin-top:2px}
.tip .ev{grid-area:ev;justify-self:end}
.tip-prob{grid-area:prob;display:grid;grid-template-columns:auto minmax(60px,140px);gap:2px 10px;align-items:center}
.tip-nums{font-size:10.5px;color:var(--muted);white-space:nowrap}
.tip-nums b{font-size:13px;color:var(--ink);margin-right:4px}
.tip .verdict{grid-area:verdict;justify-self:end}
.minibar{display:block;width:100%;height:10px}
@media (min-width:900px){
  .tip{grid-template-columns:170px minmax(0,1fr) 64px 220px 76px;grid-template-areas:"src main ev prob verdict"}
  .tip-src{flex-direction:column}
  .tip .ev{justify-self:start}
}

/* ---------- match cards ---------- */
.tourn{font-size:12px;font-weight:600;color:var(--ink-2);margin:18px 0 8px;display:flex;align-items:center;gap:10px}
.tourn::after{content:"";flex:1;height:1px;background:var(--line)}
.tourn:first-of-type{margin-top:0}
.match{margin-bottom:14px;overflow:hidden}
.match-head{padding:12px 14px 10px;border-bottom:1px solid var(--line);display:grid;gap:2px}
.match-meta{font-size:11px;color:var(--muted);display:flex;gap:10px}
.match-meta time{color:var(--ink);font-weight:600}
.match-title{font-size:17px;font-weight:700;letter-spacing:-.01em;display:flex;flex-wrap:wrap;align-items:baseline;gap:0 7px}
.match-title .vs{font-size:11px;font-weight:500;color:var(--muted)}
.odds-line{font-size:11.5px;color:var(--ink-2)}
.odds-line.none{color:var(--muted);font-style:italic}
.match-body{display:grid;gap:0}
.mcol{padding:12px 14px;border-bottom:1px solid var(--line);min-width:0}
@media (min-width:760px){
  .match-body{grid-template-columns:minmax(0,1fr) minmax(0,1fr)}
  .mcol.facts{border-right:1px solid var(--line)}
  .mcol.markets{grid-column:1 / -1}
}
@media (min-width:1180px){
  .match-body{grid-template-columns:minmax(0,1fr) minmax(0,.9fr) minmax(0,1.55fr)}
  .mcol{border-bottom:0}
  .mcol.model{border-right:1px solid var(--line)}
  .mcol.markets{grid-column:auto}
}
.form-row{display:flex;align-items:center;justify-content:space-between;gap:10px;margin-bottom:5px;font-size:12px}
.form-row span{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.form{display:flex;gap:3px;flex:none}
.form li{width:17px;height:17px;border-radius:3px;font-size:9.5px;font-weight:700;display:grid;place-items:center;color:#fff}
.form .w{background:var(--pos)} .form .d{background:var(--implied)} .form .l{background:var(--neg)}
html:has(#theme:checked) .form li{color:var(--bg)}
@media (prefers-color-scheme: dark){ .form li{color:var(--bg)} html:has(#theme:checked) .form li{color:#fff} }
.stats li{position:relative;padding-left:11px;font-size:12px;color:var(--ink-2);margin-bottom:3px}
.stats li::before{content:"";position:absolute;left:0;top:.62em;width:4px;height:4px;border-radius:50%;background:var(--line-strong)}
.stats b{color:var(--ink);font-weight:600}
.note{font-size:12px;color:var(--ink-2)}
.split{display:block;width:100%;height:12px;border-radius:3px;overflow:hidden}
.split-labels{display:flex;justify-content:space-between;gap:6px;margin-top:5px;font-size:11px;color:var(--ink-2)}
.split-labels b{color:var(--ink);font-weight:650}
.split-labels span:nth-child(2){text-align:center}
.split-labels span:last-child{text-align:right}
.caption{font-size:10.5px;color:var(--muted);margin-top:4px}
.goals{max-width:300px}
.duel-names{display:flex;justify-content:space-between;font-size:11px;font-weight:600;margin-bottom:2px}
.duel-names span:first-child{color:var(--accent)} .duel-names span:last-child{color:var(--away)}
.duel{max-width:320px;margin:0 auto}

/* markets table */
.table-wrap{overflow-x:auto;margin:0 -14px;padding:0 14px}
.mk{width:100%;min-width:520px;border-collapse:collapse;font-size:12px}
.mk th{font-size:10px;font-weight:500;color:var(--muted);text-align:right;padding:0 6px 5px;border-bottom:1px solid var(--line);white-space:nowrap}
.mk th:first-child,.mk td:first-child{text-align:left;padding-left:6px}
.mk td{text-align:right;padding:6px;border-bottom:1px solid var(--line);white-space:nowrap}
.mk td:first-child{white-space:normal;min-width:130px}
.mk tr:last-child td{border-bottom:0}
.mk .c-par{color:var(--accent);font-weight:600}
.mk .c-mod{color:var(--model)}
.mk .c-bar{width:76px}
.mk .c-bar svg{width:70px;height:10px;display:block;margin-left:auto}
.mk tr.is-pick td{background:var(--accent-soft)}
.mk tr.is-pick td:first-child{box-shadow:inset 3px 0 0 var(--accent);font-weight:600}
.match-pick{margin-top:12px;padding:10px 12px;border:1px solid var(--accent);border-left-width:3px;border-radius:6px;background:var(--accent-soft)}
.mp-top{display:flex;justify-content:space-between;align-items:center;gap:8px;flex-wrap:wrap}
.mp-top b{font-size:13px}
.mp-top .dim{font-size:11px}
.match-pick p{font-size:12px;color:var(--ink-2);margin-top:3px}
.match-src{padding:8px 14px;border-top:1px solid var(--line);font-size:10.5px;color:var(--muted);display:flex;flex-wrap:wrap;gap:2px 10px;background:var(--surface-2)}

/* método + footer */
.method{padding:14px;display:grid;gap:12px}
@media (min-width:900px){.method{grid-template-columns:repeat(3,minmax(0,1fr));gap:24px}}
.method h3{font-size:12.5px;font-weight:600;margin-bottom:4px}
.method p{font-size:12px;color:var(--ink-2);max-width:62ch}
.foot{margin-top:28px;padding:18px 0 28px;border-top:1px solid var(--line);font-size:12px;color:var(--muted)}
.foot b{color:var(--ink);font-weight:600}
.fbar{display:flex;flex-direction:column;gap:6px;padding:8px 16px}
.fgroup{display:flex;flex-wrap:nowrap;align-items:center;gap:6px;overflow-x:auto;scrollbar-width:none;max-width:100%}
.fgroup::-webkit-scrollbar{display:none}
@media (max-width:760px){.nav{position:static}.fbar{flex-direction:column;gap:6px}}
.flabel{font-size:10.5px;color:var(--muted);text-transform:uppercase;letter-spacing:.04em;margin-right:2px}
.fbar label{display:inline-flex;align-items:center;gap:5px;height:26px;padding:0 11px;border-radius:999px;border:1px solid var(--line-strong);background:var(--surface);color:var(--ink-2);font-size:11.5px;white-space:nowrap;cursor:pointer;user-select:none}
.fbar label:hover{border-color:var(--accent)}
.fbar label span{color:var(--muted);font-size:10.5px}
.nav{max-height:none}
.scards{display:grid;gap:14px}
@media (min-width:900px){.scards{grid-template-columns:repeat(2,minmax(0,1fr))}}
.scard{padding:14px 16px;display:flex;flex-direction:column;gap:6px}
.sc-top{display:flex;flex-wrap:wrap;align-items:center;gap:6px 8px;font-size:11px;color:var(--muted)}
.sc-top time{margin-left:auto;color:var(--ink);font-weight:600}
.sc-rank{display:grid;place-items:center;width:20px;height:20px;border-radius:50%;background:var(--accent-soft);color:var(--accent);font-weight:700;font-size:11px}
.sc-lg{padding:1px 8px;border-radius:999px;background:var(--surface-2);border:1px solid var(--line)}
.sc-strong{padding:1px 7px;border-radius:4px;background:var(--pos-soft);color:var(--pos);font-weight:600;font-size:10px}
.sc-match{font-size:13px;color:var(--ink-2);font-weight:500}
.sc-market{display:flex;align-items:baseline;gap:10px;flex-wrap:wrap}
.sc-market b{font-size:17px;letter-spacing:-.01em}
.sc-odds{font-size:15px;font-weight:700;color:var(--accent)}
.sc-meta{display:flex;flex-wrap:wrap;align-items:center;gap:6px 12px;font-size:11px}
.sc-stake{padding:2px 8px;border-radius:999px;background:var(--surface-2);border:1px solid var(--line);font-weight:600;color:var(--ink)}
.sc-edge{font-weight:600}
.sc-probs{display:grid;gap:4px;margin-top:4px}
.pr{display:grid;grid-template-columns:110px minmax(0,1fr) 40px;align-items:center;gap:8px;font-size:11px;color:var(--muted)}
.pr svg{width:100%;height:8px;display:block}
.pr b{color:var(--ink);text-align:right}
.scard h5{margin:8px 0 2px;font-size:10.5px;color:var(--muted);text-transform:uppercase;letter-spacing:.04em}
.trends li{display:flex;justify-content:space-between;align-items:flex-start;gap:10px;padding:4px 0;border-bottom:1px solid var(--line);font-size:12px;color:var(--ink-2)}
.trends li:last-child{border-bottom:0}
.hit{flex:none;padding:1px 7px;border-radius:999px;font-weight:700;font-size:11px}
.hit.hi{background:var(--pos-soft);color:var(--pos)}.hit.mid{background:var(--warn-soft);color:var(--warn)}.hit.lo{background:var(--neg-soft);color:var(--neg)}
.sc-why{font-size:12.5px;color:var(--ink-2);margin-top:4px}
.vs-mk li{display:grid;grid-template-columns:minmax(0,1fr) 70px 36px 56px;align-items:center;gap:8px;padding:3px 6px;font-size:11.5px;border-radius:4px}
.vs-mk li span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.vs-mk li b{text-align:right}.vs-mk li i{font-style:normal;text-align:right;font-size:11px}
.vs-mk li.mine{background:var(--accent-soft);font-weight:600}
.vs-mk .minibar{height:8px}
.sc-tg{font-size:11.5px;color:var(--muted);border-top:1px dashed var(--line);padding-top:6px;margin-top:4px}
"""#
}
