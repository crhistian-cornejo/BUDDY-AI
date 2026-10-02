import Foundation

// Real Betano Perú odds for PARLEY, from OddsPapi's v4 API (https://oddspapi.io/en/docs). Twin of
// apps/windows/src-tauri/src/services/odds.rs. Betano has no public API; OddsPapi is a third-party feed that carries
// `betano.pe` (a mirror of Betano's global prices). Nothing is asked until the user saved an OddsPapi key and
// "Cuotas Betano" is on (saving a key switches it on).
//
// A snapshot costs two to four requests: the football fixtures that are live or start within the window, the odds of the
// upcoming ones' tournaments in one call (`odds-by-tournaments`, up to 3 tournaments: it only carries matches not started
// yet), and the odds of up to 2 live matches (`odds`, one call each). Market names (`/markets`) are fetched once a week
// and kept in `MIKA/odds/`. A snapshot is reused for 10 minutes (PARLEY's chat and reviews share it), so a chatty
// afternoon doesn't eat the monthly quota. What comes back becomes a short table PARLEY reads as data.
//
// v4 only takes the key as the `apiKey` query parameter, so the URL is never logged or shown; redirects are not followed,
// so it never leaves for another host. Everything OddsPapi sends is untrusted: numbers are read without trapping, texts
// are cut and cleaned, and an error message never echoes the key.
enum Odds {
    /// The Keychain item with the user's OddsPapi key (Settings › PARLEY; the UI only asks whether it exists).
    static let keychainKey = "oddspapi-api-key"
    static let base = "https://api.oddspapi.io/v4"
    static let bookmaker = "betano.pe"
    static let football: Int64 = 10
    /// The nearest matches only: their tournaments' odds come in one (large) response.
    static let maxFixtures = 14
    static let maxTournaments = 3
    /// Live matches each cost a request of their own.
    static let maxLive = 2
    /// Markets per match in the table.
    static let maxMarkets = 8
    /// The table PARLEY gets stays short.
    static let maxText = 10_000
    static let marketsMaxAge: TimeInterval = 7 * 24 * 3600
    static let reuse: TimeInterval = 10 * 60
    /// A tournament's prices (one `odds-by-tournaments` answer) are reused this long by reviews and chat questions...
    static let tournamentReuse: Int64 = 2 * 3600
    /// ...but only this long when its next match starts within `soon` seconds: prices move most just before kick-off.
    static let tournamentReuseSoon: Int64 = 30 * 60
    static let soon: Int64 = 4 * 3600
    /// Tournaments one `odds-by-tournaments` call carries in the morning snapshot, and how many such calls it makes.
    static let tournamentBatch = 5
    static let dailyBatches = 3
    /// The morning snapshot is taken from 06:30 local time, and retried after this long when it failed.
    static let morningMinute: Int64 = 6 * 60 + 30
    static let morningRetry: TimeInterval = 15 * 60
    /// A cached tournament older than this is dropped.
    static let cacheKeep: Int64 = 12 * 3600
    /// Main markets always worth showing, whatever their `mainLine` flag says: 1X2 and both teams to score.
    static let mainMarkets: Set<Int64> = [101, 104]

    /// A match worth looking at: live, or starting within the window.
    struct Fixture: Equatable, Sendable {
        var id: String
        var home: String
        var away: String
        var tournament: String
        var tournamentID: Int64 = 0
        /// Unix seconds.
        var start: Int64
        var live: Bool
    }

    /// What a market and one of its outcomes are called.
    struct OutcomeName: Equatable, Sendable {
        var market: String
        var outcome: String
    }

    /// `MIKA/odds/`, next to the agents folder.
    static func cacheDir(agentsRoot: URL) -> URL {
        agentsRoot.deletingLastPathComponent().appendingPathComponent("odds", isDirectory: true)
    }

    // MARK: - The snapshot

    /// The last table, reused for 10 minutes when it covered the same window.
    private final class Reuse: @unchecked Sendable {
        private let lock = NSLock()
        private var last: (at: Date, end: Int64, table: String)?

        func table(end: Int64, now: Date) -> String? {
            lock.withLock {
                guard let last, now.timeIntervalSince(last.at) < Odds.reuse, abs(last.end - end) < 1800 else { return nil }
                return last.table
            }
        }

        func keep(_ table: String, end: Int64, at: Date) { lock.withLock { last = (at, end, table) } }
    }

    private static let reused = Reuse()

    /// The table for PARLEY, or why there is none (a `MessageError` in the user's words).
    static func snapshot(key: String?, now: Int64, windowEnd: Int64, timeZone: TimeZone, cacheDir: URL) async throws -> String {
        if let table = reused.table(end: windowEnd, now: Date()) { return table }
        let table = try await fetch(key: key, now: now, windowEnd: windowEnd, timeZone: timeZone, cacheDir: cacheDir)
        reused.keep(table, end: windowEnd, at: Date())
        return table
    }

    /// The morning snapshot: every football match of the local day with the prices of the busiest tournaments (up to
    /// `dailyBatches` calls of `tournamentBatch`), saved as `resumen-del-dia.md` (in `cacheDir` and every `rawDirs`) and
    /// `hoy.csv` (every active market, in `rawDirs`), so reviews and chat questions during the day reuse it. The
    /// tournaments' answers also go to the per-tournament cache, which is what makes the reviews free. Twin of
    /// `daily_snapshot` in odds.rs.
    static func dailySnapshot(key: String?, now: Int64, timeZone: TimeZone, cacheDir: URL, rawDirs: [URL]) async throws {
        let (from, to) = try manualWindow("hoy", now: now, timeZone: timeZone)
        _ = try await fetch(key: key, now: now, from: from, windowEnd: to, timeZone: timeZone, cacheDir: cacheDir,
                            daily: true, rawDirs: rawDirs)
    }

    private static func fetch(key: String?, now: Int64, from: Int64? = nil, windowEnd: Int64, timeZone: TimeZone, cacheDir: URL,
                              daily: Bool = false, rawDirs: [URL] = []) async throws -> String {
        guard let key, !key.isEmpty else { throw MessageError("Falta la clave de OddsPapi.") }
        let start = from ?? now - 3 * 3600
        let list = try await get("/fixtures?sportId=\(football)&from=\(isoUTC(start))&to=\(isoUTC(windowEnd))&hasOdds=true&bookmakers=\(bookmaker)", key: key)
        let fixtures = pickFixtures(list.json, now: now, windowEnd: windowEnd, limit: daily ? 80 : maxFixtures)
        if fixtures.isEmpty { return "No hay partidos de fútbol con cuotas de Betano en vivo ni dentro de la ventana." }
        // Upcoming matches: their tournaments' odds, five per call (that endpoint only carries matches not started yet).
        // A tournament asked for recently (the morning snapshot, an earlier review) comes from the cache instead.
        let upcoming = fixtures.filter { !$0.live && $0.tournamentID > 0 }
        var tournaments: [Int64] = []
        if daily {
            tournaments = Array(rankTournaments(upcoming).prefix(dailyBatches * tournamentBatch))
        } else {
            for f in upcoming where !tournaments.contains(f.tournamentID) && tournaments.count < maxTournaments {
                tournaments.append(f.tournamentID)
            }
        }
        var entries: [JSONValue] = []
        var cache = loadOddsCache(cacheDir)
        var toFetch: [Int64] = []
        for id in tournaments {
            let next = upcoming.filter { $0.tournamentID == id }.map(\.start).min() ?? Int64.max
            if let cached = cache[String(id)], tournamentFresh(cachedAt: cached.at, nextStart: next, now: now) {
                entries += cached.entries
            } else {
                toFetch.append(id)
            }
        }
        var fetched = false
        var begin = 0
        while begin < toFetch.count {
            let group = Array(toFetch[begin..<min(begin + tournamentBatch, toFetch.count)])
            begin += tournamentBatch
            // The fixtures endpoint asks for 2 s between calls.
            try? await Task.sleep(for: .milliseconds(2_100))
            do {
                let ids = group.map(String.init).joined(separator: ",")
                let odds = try await get("/odds-by-tournaments?tournamentIds=\(ids)&bookmaker=\(bookmaker)", key: key)
                guard case .array(let found) = odds.json else { continue }
                entries += found
                fetched = true
                let owner = Dictionary(fixtures.map { ($0.id, $0.tournamentID) }, uniquingKeysWith: { first, _ in first })
                for id in group {
                    cache[String(id)] = OddsCacheEntry(at: now, entries: found.filter { owner[$0["fixtureId"].stringValue ?? ""] == id })
                }
            } catch {
                // A later batch failing keeps what the earlier ones brought; the first failing is the snapshot's error.
                if entries.isEmpty { throw error }
                break
            }
        }
        if fetched { saveOddsCache(cache, cacheDir, now: now) }
        // Live matches: one call each, the nearest few only.
        let live = Array(fixtures.filter(\.live).prefix(maxLive))
        for f in live {
            try? await Task.sleep(for: .milliseconds(1_100))
            if let odds = try? await get("/odds?fixtureId=\(f.id)&bookmakers=\(bookmaker)", key: key) {
                switch odds.json {
                case .object: entries.append(odds.json)
                case .array(let found): entries += found
                default: break
                }
            }
        }
        let kept = fixtures.filter { f in f.live ? live.contains { $0.id == f.id } : tournaments.contains(f.tournamentID) }
        var names = loadMarkets(cacheDir)
        if names == nil {
            try? await Task.sleep(for: .milliseconds(2_100))
            if let fresh = try? await get("/markets?sportId=\(football)", key: key) {
                let found = marketNames(fresh.json)
                if !found.isEmpty { saveMarkets(footballOnly(fresh.json), cacheDir) }
                names = found
            }
        }
        if daily {
            writeDaily(kept, .array(entries), names: names ?? [:], now: now, timeZone: timeZone, cacheDir: cacheDir, rawDirs: rawDirs)
        }
        return oddsText(kept, .array(entries), names: names ?? [:], timeZone: timeZone)
    }

    // MARK: - The morning snapshot's files

    /// `resumen-del-dia.md` (a short digest to read first) in every directory, and `hoy.csv` (every active market) in the
    /// agent's. Failures to write are ignored: the snapshot still warmed the cache.
    private static func writeDaily(_ fixtures: [Fixture], _ json: JSONValue, names: [Int64: OutcomeName], now: Int64,
                                   timeZone: TimeZone, cacheDir: URL, rawDirs: [URL]) {
        let date = Date(timeIntervalSince1970: TimeInterval(now))
        let digest = oddsText(fixtures, json, names: names, timeZone: timeZone, maxMarkets: 4, maxChars: 20_000)
        let tournaments = Set(fixtures.map(\.tournamentID)).count
        let md = "# Partidos de hoy (\(ChatArchive.dateLabel(date, timeZone: timeZone)))\n\n"
            + "Consultado a las \(Picks.hour(date, timeZone: timeZone)). \(fixtures.count) partidos con cuotas de Betano, de \(tournaments) torneos. "
            + "Lee esto primero; hoy.csv trae todos los mercados. Usa la web solo para validar bajas o forma de un partido concreto.\n\n\(digest)\n"
        let csv = "\u{FEFF}" + oddsCSV(fixtures, json, names: names, timeZone: timeZone)
        for dir in [cacheDir] + rawDirs {
            try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            try? Data(md.utf8).write(to: dir.appendingPathComponent("resumen-del-dia.md"), options: .atomic)
        }
        for dir in rawDirs { try? Data(csv.utf8).write(to: dir.appendingPathComponent("hoy.csv"), options: .atomic) }
        // The day's match list, for the free form stats (FreeStats.swift). Football only: this layer has no tennis.
        try? FreeStats.fixturesJSON(day: ChatArchive.dateLabel(date, timeZone: timeZone), fixtures: fixtures)
            .write(to: cacheDir.appendingPathComponent("daily-fixtures.json"), options: .atomic)
    }

    /// `partido,torneo,hora,mercado,seleccion,cuota`: every active price of every market, no player props.
    static func oddsCSV(_ fixtures: [Fixture], _ json: JSONValue, names: [Int64: OutcomeName], timeZone: TimeZone) -> String {
        var byID: [String: JSONValue] = [:]
        if case .array(let entries) = json {
            for entry in entries { if let id = entry["fixtureId"].stringValue { byID[id] = entry } }
        }
        func cell(_ text: String) -> String {
            var t = text.replacingOccurrences(of: "\n", with: " ").replacingOccurrences(of: "\r", with: " ")
            // A cell starting with one of these would be a formula in a spreadsheet.
            if let first = t.first, "=+-@".contains(first) { t = "'" + t }
            return "\"" + t.replacingOccurrences(of: "\"", with: "\"\"") + "\""
        }
        var out = "partido,torneo,hora,mercado,seleccion,cuota\n"
        for fixture in fixtures {
            guard let entry = byID[fixture.id], case .object(let markets) = entry["bookmakerOdds"][bookmaker]["markets"] else { continue }
            let when = fixture.live ? "EN VIVO" : Picks.hour(fixture.start, timeZone: timeZone)
            for (marketKey, market) in markets.sorted(by: { $0.key < $1.key }) {
                if market["marketActive"] == .bool(false) { continue }
                let marketID = Int64(marketKey) ?? 0
                guard case .object(let outcomes) = market["outcomes"] else { continue }
                for (outcomeKey, outcome) in outcomes.sorted(by: { $0.key < $1.key }) {
                    let quote = outcome["players"]["0"]
                    if quote == .null || quote["active"] == .bool(false) { continue }
                    guard let outcomeID = Int64(outcomeKey), case .number(let price) = quote["price"], price >= 1.01, price < 1000 else { continue }
                    let name = names[outcomeID] ?? knownOutcome(outcomeID) ?? OutcomeName(market: "Mercado \(marketID)", outcome: "Opción \(outcomeID)")
                    let row = [cell("\(fixture.home) vs \(fixture.away)"), cell(fixture.tournament), cell(when), cell(name.market),
                               cell(name.outcome), String(format: "%.2f", price)]
                    out += row.joined(separator: ",") + "\n"
                }
            }
        }
        return out
    }

    // MARK: - Each tournament's last answer

    struct OddsCacheEntry: Codable, Equatable, Sendable {
        /// Unix seconds.
        var at: Int64
        var entries: [JSONValue]
    }

    /// How long a tournament's prices are reused: 2 hours, 30 minutes when its next match starts within 4 hours.
    static func reuseSeconds(nextStart: Int64, now: Int64) -> Int64 {
        nextStart - now < soon ? tournamentReuseSoon : tournamentReuse
    }

    static func tournamentFresh(cachedAt: Int64, nextStart: Int64, now: Int64) -> Bool {
        let age = now - cachedAt
        return age >= 0 && age < reuseSeconds(nextStart: nextStart, now: now)
    }

    /// Tournaments with most matches first, then the earliest kick-off: five of them travel in one call.
    static func rankTournaments(_ fixtures: [Fixture]) -> [Int64] {
        var count: [Int64: Int] = [:]
        var first: [Int64: Int64] = [:]
        for f in fixtures where f.tournamentID > 0 {
            count[f.tournamentID, default: 0] += 1
            first[f.tournamentID] = min(first[f.tournamentID] ?? Int64.max, f.start)
        }
        return count.keys.sorted { a, b in
            if count[a] != count[b] { return (count[a] ?? 0) > (count[b] ?? 0) }
            if first[a] != first[b] { return (first[a] ?? 0) < (first[b] ?? 0) }
            return a < b
        }
    }

    private static func oddsCacheURL(_ cacheDir: URL) -> URL { cacheDir.appendingPathComponent("odds-cache.json") }

    static func loadOddsCache(_ cacheDir: URL) -> [String: OddsCacheEntry] {
        (try? Data(contentsOf: oddsCacheURL(cacheDir))).flatMap { try? JSONDecoder().decode([String: OddsCacheEntry].self, from: $0) } ?? [:]
    }

    private static func saveOddsCache(_ cache: [String: OddsCacheEntry], _ cacheDir: URL, now: Int64) {
        let kept = cache.filter { now - $0.value.at < cacheKeep }
        guard let data = try? JSONEncoder().encode(kept) else { return }
        try? FileManager.default.createDirectory(at: cacheDir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try? data.write(to: oddsCacheURL(cacheDir), options: .atomic)
    }

    // MARK: - When the morning snapshot is due

    /// The local day (`2026-10-01`) when the day's snapshot should be taken now: it is 06:30 or later and `marker` (the text
    /// of `odds/daily.json`) does not already name the day. Nil otherwise.
    static func morningDue(now: Date, timeZone: TimeZone, marker: String?) -> String? {
        let local = Int64(now.timeIntervalSince1970.rounded(.down)) + Int64(timeZone.secondsFromGMT(for: now))
        let minute = ((local % 86_400) + 86_400) % 86_400 / 60
        guard minute >= morningMinute else { return nil }
        let day = ChatArchive.dateLabel(now, timeZone: timeZone)
        if let marker, marker.contains(day) { return nil }
        return day
    }

    // MARK: - Calls made today

    private static let usageDayKey = "mika.oddspapi.usage.day"
    private static let usageCallsKey = "mika.oddspapi.usage.calls"

    /// Counts one request to OddsPapi today (the local day), for the usage line.
    static func noteCall(now: Date = Date(), defaults: UserDefaults = .standard) {
        let day = ChatArchive.dateLabel(now)
        let calls = defaults.string(forKey: usageDayKey) == day ? defaults.integer(forKey: usageCallsKey) : 0
        defaults.set(day, forKey: usageDayKey)
        defaults.set(calls + 1, forKey: usageCallsKey)
    }

    /// Requests made to OddsPapi today (the local day).
    static func callsToday(now: Date = Date(), defaults: UserDefaults = .standard) -> Int {
        defaults.string(forKey: usageDayKey) == ChatArchive.dateLabel(now) ? defaults.integer(forKey: usageCallsKey) : 0
    }

    // MARK: - Requests

    private static let session: URLSession = {
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 40
        config.timeoutIntervalForResource = 40
        return URLSession(configuration: config)
    }()

    /// A redirect is answered as it comes (and then refused as a status), never followed with the key.
    private final class NoRedirects: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
        func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                        newRequest request: URLRequest) async -> URLRequest? { nil }
    }

    private static func get(_ path: String, key: String) async throws -> (json: JSONValue, data: Data) {
        let encodedKey = key.addingPercentEncoding(withAllowedCharacters: .alphanumerics) ?? ""
        let separator = path.contains("?") ? "&" : "?"
        guard let url = URL(string: base + path + separator + "apiKey=" + encodedKey) else {
            throw MessageError("No se pudo preparar la conexión.")
        }
        var request = URLRequest(url: url, timeoutInterval: 40)
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        let reply: (Data, URLResponse)
        do {
            noteCall()
            reply = try await session.data(for: request, delegate: NoRedirects())
        } catch {
            throw MessageError("Sin conexión con OddsPapi.")
        }
        let status = (reply.1 as? HTTPURLResponse)?.statusCode ?? 0
        if status == 401 || status == 403 { throw MessageError("OddsPapi no acepta la clave (¿es de la API v4?).") }
        if status == 429 { throw MessageError("OddsPapi: se acabó la cuota de consultas por ahora.") }
        guard (200..<300).contains(status) else { throw MessageError(statusMessage(status, body: reply.0, key: key)) }
        guard let json = try? JSONDecoder().decode(JSONValue.self, from: reply.0) else {
            throw MessageError("OddsPapi devolvió datos que no se entienden.")
        }
        return (json, reply.0)
    }

    /// OddsPapi says why in a short JSON message; nothing that could hold the key is ever repeated.
    static func statusMessage(_ status: Int, body: Data, key: String) -> String {
        let json = try? JSONDecoder().decode(JSONValue.self, from: body)
        var detail = json?["message"].stringValue ?? json?["error"].stringValue ?? ""
        if !key.isEmpty { detail = detail.replacingOccurrences(of: key, with: "") }
        detail = String(detail.prefix(120))
        return "OddsPapi respondió \(status)\(detail.isEmpty ? "" : ": ")\(detail)."
    }

    // MARK: - Times

    /// `2026-10-01T13:30:00Z`.
    static func isoUTC(_ seconds: Int64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(max(0, seconds)))
        let t = ((max(0, seconds) % 86_400) + 86_400) % 86_400
        func two(_ n: Int64) -> String { n < 10 ? "0\(n)" : "\(n)" }
        return "\(ChatArchive.dateLabel(date, timeZone: TimeZone(secondsFromGMT: 0)!))T\(two(t / 3600)):\(two((t % 3600) / 60)):\(two(t % 60))Z"
    }

    /// `2026-10-01T13:30:00.000Z` (or with `+hh:mm`) → Unix seconds.
    static func parseISO(_ text: String) -> Int64? {
        let b = Array(text.utf8)
        guard b.count >= 19, b[4] == 0x2D, b[7] == 0x2D, b[10] == 0x54 || b[10] == 0x20, b[13] == 0x3A, b[16] == 0x3A else { return nil }
        func number(_ range: Range<Int>) -> Int64? {
            guard range.upperBound <= b.count else { return nil }
            return Int64(String(decoding: b[range], as: UTF8.self))
        }
        guard let y = number(0..<4), let m = number(5..<7), let d = number(8..<10),
              let hh = number(11..<13), let mm = number(14..<16), let ss = number(17..<19),
              (1...12).contains(m), (1...31).contains(d), hh <= 23, mm <= 59, ss <= 60 else { return nil }
        // Days from civil (Howard Hinnant).
        let y2 = m <= 2 ? y - 1 : y
        let era = (y2 >= 0 ? y2 : y2 - 399) / 400
        let yoe = y2 - era * 400
        let mp = (m + 9) % 12
        let doy = (153 * mp + 2) / 5 + d - 1
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy
        let days = era * 146_097 + doe - 719_468
        var seconds = days * 86_400 + hh * 3600 + mm * 60 + ss
        var rest = Array(b[19...])
        while let first = rest.first, first == 0x2E || (0x30...0x39).contains(first) { rest.removeFirst() }
        if let sign = rest.first, sign == 0x2B || sign == 0x2D {
            let offset = Array(rest.dropFirst())
            guard offset.count >= 2, let oh = Int64(String(decoding: offset[0..<2], as: UTF8.self)) else { return nil }
            let om = offset.count >= 5 ? Int64(String(decoding: offset[3..<5], as: UTF8.self)) ?? 0 : 0
            seconds -= (sign == 0x2B ? 1 : -1) * (oh * 3600 + om * 60)
        }
        return seconds
    }

    // MARK: - A question's own range

    /// Lower case without accents, for matching the user's words.
    static func normalized(_ text: String) -> String {
        String(text.lowercased().map { c -> Character in
            switch c {
            case "á": return "a"
            case "é": return "e"
            case "í": return "i"
            case "ó": return "o"
            case "ú", "ü": return "u"
            case "ñ": return "n"
            default: return c
            }
        })
    }

    /// The day or hours a chat question asks about (hoy by default, mañana, ayer, pasado mañana, AAAA-MM-DD or
    /// DD/MM/AAAA, "próximas N horas" up to 36), as Unix seconds. Vague words (semana, mes…) ask for precision instead of
    /// silently using the settings. Twin of `manual_window` in odds.rs.
    static func manualWindow(_ query: String, now: Int64, timeZone: TimeZone) throws -> (from: Int64, to: Int64) {
        let offset = Int64(timeZone.secondsFromGMT(for: Date(timeIntervalSince1970: TimeInterval(now))))
        let text = normalized(query)
        let local = now + offset
        let day = (local >= 0 ? local / 86_400 : (local - 86_399) / 86_400) * 86_400 - offset
        let separators = CharacterSet.whitespacesAndNewlines.union(CharacterSet(charactersIn: ",.?!;:"))
        let tokens = text.components(separatedBy: separators).filter { !$0.isEmpty }
        var dates: [Int64] = []
        for token in tokens {
            let b = Array(token.utf8)
            var date: String?
            if b.count == 10, b[4] == 0x2D, b[7] == 0x2D { date = token }
            else if b.count == 10, b[2] == 0x2F, b[5] == 0x2F { date = "\(token.suffix(4))-\(token.dropFirst(3).prefix(2))-\(token.prefix(2))" }
            guard let date else { continue }
            guard let utc = parseISO("\(date)T00:00:00Z"), isoUTC(utc).hasPrefix(date) else {
                throw MessageError("Fecha inválida: \(date). Indica la fecha como AAAA-MM-DD.")
            }
            dates.append(utc - offset)
        }
        if dates.count > 1 { throw MessageError("La consulta indica varias fechas. Precisa un día o un rango en horas para esta consulta de cuotas.") }
        if let date = dates.first { return (date, date + 86_400) }
        if text.contains("proximas") || text.contains("proximos"),
           let i = tokens.firstIndex(where: { ["horas", "hora", "h"].contains($0) }), i > 0,
           let hours = Int64(tokens[i - 1]), (1...36).contains(hours) {
            return (now, now + hours * 3600)
        }
        if text.contains("pasado manana") || text.contains("pasadomanana") { return (day + 2 * 86_400, day + 3 * 86_400) }
        if tokens.contains("manana"), !tokens.contains("hoy") { return (day + 86_400, day + 2 * 86_400) }
        if tokens.contains("ayer") { return (day - 86_400, day) }
        if text.contains("semana") || text.contains("mes") || text.contains("proxim") || (tokens.contains("manana") && tokens.contains("hoy")) {
            throw MessageError("Precisa el día (hoy, mañana o AAAA-MM-DD) o las próximas N horas, hasta 36. La ventana automática no limita esta petición.")
        }
        return (day, day + 86_400)
    }

    // MARK: - Reading what came back

    /// Live matches first, then by kick-off; finished, cancelled and out-of-window ones are left out.
    static func pickFixtures(_ json: JSONValue, now: Int64, windowEnd: Int64, limit: Int = maxFixtures) -> [Fixture] {
        guard case .array(let list) = json else { return [] }
        var out: [Fixture] = []
        for f in list {
            let status = int(f["statusId"]) ?? int(f["status"]["statusId"]) ?? 0
            let live = status == 1 || f["status"]["live"] == .bool(true)
            guard let start = int(f["startTime"]) ?? f["startTime"].stringValue.flatMap(parseISO) else { continue }
            if status >= 2 || !(live || (start >= now - 300 && start <= windowEnd)) { continue }
            guard let id = f["fixtureId"].stringValue, !id.isEmpty, id.utf8.count <= 40,
                  id.utf8.allSatisfy({ (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) })
            else { continue }
            func pick(_ a: JSONValue, _ b: JSONValue) -> String {
                let first = text(a)
                return first.isEmpty ? text(b) : first
            }
            let tournament = [pick(f["tournamentName"], f["tournament"]["tournamentName"]),
                              pick(f["categoryName"], f["tournament"]["categoryName"])]
                .filter { !$0.isEmpty }.joined(separator: ", ")
            out.append(Fixture(id: id, home: pick(f["participant1Name"], f["participants"]["participant1Name"]),
                               away: pick(f["participant2Name"], f["participants"]["participant2Name"]),
                               tournament: tournament, tournamentID: int(f["tournamentId"]) ?? 0, start: start, live: live))
        }
        out.sort { ($0.live ? 0 : 1, $0.start) < ($1.live ? 0 : 1, $1.start) }
        return Array(out.prefix(limit))
    }

    /// outcomeId → (market name, outcome name), from `/markets`. Player props are left out; a market that exists for
    /// every line ("Over Under Full Time") carries its line in the name.
    static func marketNames(_ json: JSONValue) -> [Int64: OutcomeName] {
        guard case .array(let markets) = json else { return [:] }
        var out: [Int64: OutcomeName] = [:]
        for market in markets {
            if market["playerProp"] == .bool(true) { continue }
            let base = market["marketName"].stringValue ?? ""
            var name = base
            if case .number(let h) = market["handicap"], h != 0, h.isFinite {
                let line = h.rounded() == h && abs(h) < 1e15 ? String(Int64(h)) : String(h)
                name = "\(base) \(line)"
            }
            guard case .array(let outcomes) = market["outcomes"] else { continue }
            for outcome in outcomes {
                if let id = int(outcome["outcomeId"]) {
                    out[id] = OutcomeName(market: name, outcome: outcome["outcomeName"].stringValue ?? "")
                }
            }
        }
        return out
    }

    /// The football markets everyone knows, for when `/markets` could not be read.
    static func knownOutcome(_ id: Int64) -> OutcomeName? {
        switch id {
        case 101: return OutcomeName(market: "Resultado final", outcome: "1")
        case 102: return OutcomeName(market: "Resultado final", outcome: "X")
        case 103: return OutcomeName(market: "Resultado final", outcome: "2")
        case 104: return OutcomeName(market: "Ambos marcan", outcome: "Sí")
        case 105: return OutcomeName(market: "Ambos marcan", outcome: "No")
        case 1010: return OutcomeName(market: "Más/Menos 2.5 goles", outcome: "Más")
        case 1011: return OutcomeName(market: "Más/Menos 2.5 goles", outcome: "Menos")
        default: return nil
        }
    }

    /// One block per match: `- Local vs Visita (torneo) · 20:30` and its main markets, `Mercado: lado @cuota · …`.
    /// Only active prices, no player props; a suspended book (a live match between plays) says so.
    static func oddsText(_ fixtures: [Fixture], _ json: JSONValue, names: [Int64: OutcomeName], timeZone: TimeZone,
                         maxMarkets: Int = Odds.maxMarkets, maxChars: Int = Odds.maxText) -> String {
        var byID: [String: JSONValue] = [:]
        if case .array(let entries) = json {
            for entry in entries { if let id = entry["fixtureId"].stringValue { byID[id] = entry } }
        }
        var out = ""
        for fixture in fixtures {
            guard let entry = byID[fixture.id] else { continue }
            let book = entry["bookmakerOdds"][bookmaker]
            // The main markets (1X2, both teams to score) first, then the main lines of the others, each by name.
            var main: [String: [String]] = [:]
            var other: [String: [String]] = [:]
            var seen = Set<String>()
            if case .object(let markets) = book["markets"] {
                // In key order, like serde_json's map on Windows.
                for (marketKey, market) in markets.sorted(by: { $0.key < $1.key }) {
                    if market["marketActive"] == .bool(false) { continue }
                    let marketID = Int64(marketKey) ?? 0
                    guard case .object(let outcomes) = market["outcomes"] else { continue }
                    for (outcomeKey, outcome) in outcomes.sorted(by: { $0.key < $1.key }) {
                        let quote = outcome["players"]["0"]
                        if quote == .null || quote["active"] == .bool(false) { continue }
                        guard let outcomeID = Int64(outcomeKey), case .number(let price) = quote["price"],
                              price >= 1.01, price < 1000 else { continue }
                        guard mainMarkets.contains(marketID) || quote["mainLine"] == .bool(true),
                              let name = names[outcomeID] ?? knownOutcome(outcomeID),
                              seen.insert("\(name.market)|\(name.outcome)").inserted else { continue }
                        let side = "\(name.outcome) @\(String(format: "%.2f", price))"
                        if mainMarkets.contains(marketID) { main[name.market, default: []].append(side) }
                        else { other[name.market, default: []].append(side) }
                    }
                }
            }
            let lines = (main.keys.sorted().map { ($0, main[$0] ?? []) } + other.keys.sorted().map { ($0, other[$0] ?? []) })
                .prefix(maxMarkets)
            if lines.isEmpty { continue }
            let when = fixture.live ? "EN VIVO" : Picks.hour(fixture.start, timeZone: timeZone)
            let suspended = book["suspended"] == .bool(true) && fixture.live ? " (cuotas suspendidas en este momento)" : ""
            let tournament = fixture.tournament.isEmpty ? "" : " (" + fixture.tournament + ")"
            var block = "- \(fixture.home) vs \(fixture.away)\(tournament) · \(when)\(suspended)\n"
            for (market, sides) in lines { block += "  \(market): \(sides.joined(separator: " · "))\n" }
            if out.utf8.count + block.utf8.count > maxChars { break }
            out += block
        }
        return out.isEmpty ? "OddsPapi no devolvió cuotas activas de Betano para los partidos de la ventana." : out
    }

    /// The catalogue comes for every sport (≈ 9 MB); only football is kept.
    static func footballOnly(_ json: JSONValue) -> JSONValue {
        guard case .array(let markets) = json else { return .array([]) }
        return .array(markets.filter { int($0["sportId"]) == football })
    }

    /// An integer, only when the number is one (never traps on a huge or fractional value).
    private static func int(_ value: JSONValue) -> Int64? {
        if case .number(let n) = value { return Int64(exactly: n) }
        return nil
    }

    /// A name from the feed: no control characters, at most 60 characters.
    private static func text(_ value: JSONValue) -> String {
        var scalars = String.UnicodeScalarView()
        var count = 0
        for scalar in (value.stringValue ?? "").unicodeScalars where scalar.properties.generalCategory != .control {
            if count == 60 { break }
            scalars.append(scalar)
            count += 1
        }
        return String(scalars)
    }

    // MARK: - The market names, cached for a week

    private static func marketsURL(_ cacheDir: URL) -> URL { cacheDir.appendingPathComponent("markets-v4-\(football).json") }

    static func loadMarkets(_ cacheDir: URL, now: Date = Date()) -> [Int64: OutcomeName]? {
        let url = marketsURL(cacheDir)
        guard let modified = (try? url.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate,
              now.timeIntervalSince(modified) < marketsMaxAge,
              let data = try? Data(contentsOf: url),
              let json = try? JSONDecoder().decode(JSONValue.self, from: data) else { return nil }
        let names = marketNames(json)
        return names.isEmpty ? nil : names
    }

    private static func saveMarkets(_ json: JSONValue, _ cacheDir: URL) {
        guard let data = try? JSONEncoder().encode(json) else { return }
        try? FileManager.default.createDirectory(at: cacheDir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try? data.write(to: marketsURL(cacheDir), options: .atomic)
    }
}
