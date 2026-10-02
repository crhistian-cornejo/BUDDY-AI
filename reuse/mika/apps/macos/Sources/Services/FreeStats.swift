import Foundation

// Free team and player form for PARLEY, from two keyless public sources: Football-Data.co.uk (football results CSVs)
// and TennisMyLife (tennis match CSVs). Twin of apps/windows/src-tauri/src/services/stats.rs. Off until the user turns
// on `useFreeStats`. Once a day, after the morning odds snapshot, each file is requested at most once (conditional GET
// with ETag / Last-Modified, 0.9 s between files, `User-Agent: MIKA`), parsed, reduced to per-team / per-player form and
// cached in `MIKA/odds/stats-cache.json`. The form of today's fixtures is appended to `resumen-del-dia.md`
// ("## Forma reciente") and written to `stats-hoy.csv`. A team or player that cannot be matched with certainty is
// reported as "sin datos"; names are never guessed (exact match after normalisation and a fixed alias table, nothing
// fuzzy). See docs/PARLEY-DATA.md.
//
// Gap: the macOS odds layer (Odds.swift) only lists football, so `odds/daily-fixtures.json` has football fixtures only
// and the tennis code below has nothing to match until that layer carries tennis too.
enum FreeStats {
    static let fdBase = "https://football-data.co.uk"
    static let tmlBase = "https://stats.tennismylife.org/data"
    static let userAgent = "MIKA"
    /// Pause between two requests, in milliseconds.
    static let gapMs = 900
    static let maxBytes = 30_000_000
    /// After a failed run, wait this long before asking again.
    static let retry: TimeInterval = 15 * 60

    /// Football-Data's main leagues: file code and readable name. `/mmz4281/<season>/<code>.csv`.
    static let mainLeagues: [(code: String, name: String)] = [
        ("E0", "Premier League"), ("E1", "Championship"), ("SP1", "La Liga"), ("D1", "Bundesliga"), ("I1", "Serie A"), ("F1", "Ligue 1"),
        ("N1", "Eredivisie"), ("P1", "Liga Portugal"), ("B1", "Pro League"), ("T1", "Süper Lig"), ("SC0", "Scottish Premiership"), ("G1", "Super League Grecia"),
    ]
    /// Football-Data's "extra leagues": `/new/<code>.csv`. Peru, Colombia and Chile have no file (COL/CHL answer 200 with
    /// Poland's/China's data: never used).
    static let extraCountries: [String] = ["ARG", "BRA", "MEX", "USA", "JPN", "CHN", "DNK", "NOR", "SWE", "RUS", "FIN", "IRL", "AUT", "POL", "ROU", "SWZ"]

    /// Nothing found for a name: the reason, in Spanish.
    struct Miss: Error, Equatable { var reason: String }

    // MARK: - CSV and dates

    /// RFC 4180 reader: BOM, quoted fields with `""`, commas and line breaks inside quotes, CRLF, blank lines.
    static func parseCSV(_ input: String) -> [[String]] {
        var scalars = Array(input.unicodeScalars)
        let bom: Unicode.Scalar = "\u{FEFF}"
        let quote: Unicode.Scalar = "\""
        let comma: Unicode.Scalar = ","
        let cr: Unicode.Scalar = "\r"
        let lf: Unicode.Scalar = "\n"
        if scalars.first == bom { scalars.removeFirst() }
        var rows: [[String]] = []
        var row: [String] = []
        var field = String.UnicodeScalarView()
        var quoted = false
        var i = 0
        while i < scalars.count {
            let c = scalars[i]
            if quoted {
                if c == quote {
                    if i + 1 < scalars.count, scalars[i + 1] == quote { field.append(quote); i += 1 } else { quoted = false }
                } else {
                    field.append(c)
                }
            } else if c == quote && field.isEmpty {
                quoted = true
            } else if c == comma {
                row.append(String(field)); field = String.UnicodeScalarView()
            } else if c == cr {
                // ignored: CRLF line ends
            } else if c == lf {
                row.append(String(field)); rows.append(row); row = []; field = String.UnicodeScalarView()
            } else {
                field.append(c)
            }
            i += 1
        }
        if !field.isEmpty || !row.isEmpty { row.append(String(field)); rows.append(row) }
        return rows.filter { !($0.count == 1 && $0[0].trimmingCharacters(in: .whitespacesAndNewlines).isEmpty) }
    }

    /// A CSV with a header row; missing columns read as "".
    struct Table {
        var index: [String: Int] = [:]
        var rows: [[String]] = []

        init(_ text: String) {
            var all = FreeStats.parseCSV(text)
            let header: [String] = all.isEmpty ? [] : all.removeFirst()
            for (i, h) in header.enumerated() {
                let name = h.trimmingCharacters(in: .whitespacesAndNewlines)
                if index[name] == nil { index[name] = i }
            }
            rows = all
        }

        func has(_ columns: [String]) -> Bool { columns.allSatisfy { index[$0] != nil } }

        func get(_ row: [String], _ column: String) -> String {
            guard let i = index[column], i < row.count else { return "" }
            return row[i].trimmingCharacters(in: .whitespacesAndNewlines)
        }
    }

    /// Days since 1970-01-01 of a civil date (Howard Hinnant).
    static func daysFromCivil(_ year: Int, _ month: Int, _ day: Int) -> Int {
        let y = month <= 2 ? year - 1 : year
        let era = (y >= 0 ? y : y - 399) / 400
        let yoe = y - era * 400
        let doy = (153 * (month > 2 ? month - 3 : month + 9) + 2) / 5 + day - 1
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy
        return era * 146_097 + doe - 719_468
    }

    static func civilFromDays(_ days: Int) -> (year: Int, month: Int, day: Int) {
        let z = days + 719_468
        let era = (z >= 0 ? z : z - 146_096) / 146_097
        let doe = z - era * 146_097
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100)
        let mp = (5 * doy + 2) / 153
        let d = doy - (153 * mp + 2) / 5 + 1
        let m = mp < 10 ? mp + 3 : mp - 9
        return (yoe + era * 400 + (m <= 2 ? 1 : 0), m, d)
    }

    /// `dd/mm/yyyy`, `dd/mm/yy` (old Football-Data files), `yyyy-mm-dd` or `yyyymmdd` → days since the epoch.
    static func parseDate(_ text: String) -> Int? {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        func ok(_ y: Int?, _ m: Int?, _ d: Int?) -> Int? {
            guard let y, let m, let d, (1...12).contains(m), (1...31).contains(d), (1900...2200).contains(y) else { return nil }
            return daysFromCivil(y, m, d)
        }
        if t.utf8.count == 8, t.utf8.allSatisfy({ $0 >= 48 && $0 <= 57 }) {
            let c = Array(t)
            return ok(Int(String(c[0..<4])), Int(String(c[4..<6])), Int(String(c[6..<8])))
        }
        if t.contains("/") {
            let p = t.components(separatedBy: "/")
            guard p.count == 3, let d = Int(p[0]), let m = Int(p[1]), var y = Int(p[2]) else { return nil }
            if y < 100 { y += y < 70 ? 2000 : 1900 }
            return ok(y, m, d)
        }
        let p = t.components(separatedBy: "-")
        if p.count == 3 { return ok(Int(p[0]), Int(p[1]), Int(p[2])) }
        return nil
    }

    private static func two(_ n: Int) -> String { n < 10 ? "0\(n)" : "\(n)" }

    private static func four(_ n: Int) -> String {
        let s = String(n)
        return String(repeating: "0", count: max(0, 4 - s.count)) + s
    }

    static func iso(_ days: Int) -> String {
        let c = civilFromDays(days)
        return four(c.year) + "-" + two(c.month) + "-" + two(c.day)
    }

    static func dmy(_ days: Int) -> String {
        let c = civilFromDays(days)
        return two(c.day) + "/" + two(c.month) + "/" + four(c.year)
    }

    private static func mod100(_ n: Int) -> Int { ((n % 100) + 100) % 100 }

    /// Football-Data's season folder for a date: seasons start in July (`2526` from 2025-07 to 2026-06).
    static func seasonCode(year: Int, month: Int) -> String {
        let start = month >= 7 ? year : year - 1
        return two(mod100(start)) + two(mod100(start + 1))
    }

    static func previousSeason(_ code: String) -> String {
        let start = Int(code.prefix(2)) ?? 0
        return two(mod100(start - 1)) + two(mod100(start))
    }

    // MARK: - Football

    struct FMatch: Equatable, Sendable {
        var day: Int
        var home: String
        var away: String
        var hg: Int
        var ag: Int
        var season: String
        var league: String
    }

    /// Played matches of a Football-Data file: main layout (`HomeTeam, AwayTeam, FTHG, FTAG`) or extra-leagues layout
    /// (`Home, Away, HG, AG, Season, League`). Unplayed rows (no score) and rows with a broken date are skipped.
    static func parseFootball(_ text: String, defaultLeague: String) throws -> [FMatch] {
        let table = Table(text)
        let cols: (home: String, away: String, hg: String, ag: String)
        if table.has(["HomeTeam", "AwayTeam", "FTHG", "FTAG"]) {
            cols = ("HomeTeam", "AwayTeam", "FTHG", "FTAG")
        } else if table.has(["Home", "Away", "HG", "AG"]) {
            cols = ("Home", "Away", "HG", "AG")
        } else {
            throw MessageError("Football-Data: el archivo no tiene las columnas esperadas.")
        }
        guard table.has(["Date"]) else { throw MessageError("Football-Data: falta la columna Date.") }
        var out: [FMatch] = []
        for row in table.rows {
            guard let day = parseDate(table.get(row, "Date")), let hg = Int(table.get(row, cols.hg)), let ag = Int(table.get(row, cols.ag)),
                  hg >= 0, ag >= 0 else { continue }
            let home = table.get(row, cols.home)
            let away = table.get(row, cols.away)
            if home.isEmpty || away.isEmpty { continue }
            let league: String
            if table.has(["League"]) {
                let country = table.get(row, "Country")
                let name = table.get(row, "League")
                league = country.isEmpty ? name : "\(country) \(name)"
            } else {
                league = defaultLeague
            }
            out.append(FMatch(day: day, home: home, away: away, hg: hg, ag: ag, season: table.get(row, "Season"), league: league))
        }
        return out
    }

    struct Split: Equatable, Sendable, Codable {
        var form = ""
        var pts = 0
        var n = 0
    }

    struct TeamForm: Equatable, Sendable, Codable {
        var name = ""
        var league = ""
        /// Cache key of the file it came from; two files with the same canonical name make a name ambiguous.
        var source = ""
        /// Last 5 results, oldest to newest (the last letter is the latest match).
        var form5 = ""
        var gf5 = 0
        var ga5 = 0
        var n5 = 0
        /// Last 5 as home team and last 5 as away team.
        var home = Split()
        var away = Split()
        /// Points per game in the team's current season (all the file's competitions) and games played.
        var ppg: Double = 0
        var played = 0
        /// Date of the latest match (ISO).
        var last = ""
    }

    private static func letter(_ gf: Int, _ ga: Int) -> Character { gf > ga ? "W" : (gf == ga ? "D" : "L") }
    private static func points(_ c: Character) -> Int { c == "W" ? 3 : (c == "D" ? 1 : 0) }

    /// Form of every team in a list of matches (any order). Teams come out sorted by name.
    static func footballForms(_ matches: [FMatch], source: String) -> [TeamForm] {
        struct Entry { var day: Int; var idx: Int; var home: Bool; var gf: Int; var ga: Int; var season: String; var league: String }
        var byTeam: [String: [Entry]] = [:]
        for (idx, m) in matches.enumerated() {
            byTeam[m.home, default: []].append(Entry(day: m.day, idx: idx, home: true, gf: m.hg, ga: m.ag, season: m.season, league: m.league))
            byTeam[m.away, default: []].append(Entry(day: m.day, idx: idx, home: false, gf: m.ag, ga: m.hg, season: m.season, league: m.league))
        }
        func split(_ entries: [Entry]) -> Split {
            let tail = Array(entries.suffix(5))
            let form = String(tail.map { letter($0.gf, $0.ga) })
            return Split(form: form, pts: form.reduce(0) { $0 + points($1) }, n: tail.count)
        }
        var out: [TeamForm] = []
        for name in byTeam.keys.sorted() {
            let all = (byTeam[name] ?? []).sorted { a, b in a.day != b.day ? a.day < b.day : a.idx < b.idx }
            guard let last = all.last else { continue }
            let tail = Array(all.suffix(5))
            let season = all.filter { $0.season == last.season }
            let seasonPts = season.reduce(0) { $0 + points(letter($1.gf, $1.ga)) }
            let ppg = (Double(seasonPts) / Double(season.count) * 100).rounded() / 100
            out.append(TeamForm(
                name: name, league: last.league, source: source,
                form5: String(tail.map { letter($0.gf, $0.ga) }),
                gf5: tail.reduce(0) { $0 + $1.gf }, ga5: tail.reduce(0) { $0 + $1.ga }, n5: tail.count,
                home: split(all.filter { $0.home }), away: split(all.filter { !$0.home }),
                ppg: ppg, played: season.count, last: iso(last.day)))
        }
        return out
    }

    private static let foldSpecials: [Unicode.Scalar: String] = {
        var map: [Unicode.Scalar: String] = [:]
        func add(_ chars: String, _ to: String) { for c in chars.unicodeScalars { map[c] = to } }
        add("àáâãäåāăą", "a"); add("çćč", "c"); add("èéêëēėęě", "e"); add("ìíîïīı", "i"); add("ñńň", "n")
        add("òóôõöøőō", "o"); add("ùúûüūůű", "u"); add("ýÿ", "y"); add("šśş", "s"); add("žźż", "z"); add("ł", "l")
        add("đð", "d"); add("ğ", "g"); add("ß", "ss"); add("'’`´", ""); add("&", " and ")
        return map
    }()

    /// Lower case, no accents, letters and digits separated by single spaces (apostrophes vanish).
    static func fold(_ text: String) -> String {
        var out = ""
        for scalar in text.lowercased().unicodeScalars {
            if let special = foldSpecials[scalar] {
                out += special
            } else if scalar.properties.isAlphabetic || scalar.properties.numericType != nil {
                out.unicodeScalars.append(scalar)
            } else {
                out += " "
            }
        }
        return out.split(separator: " ", omittingEmptySubsequences: true).joined(separator: " ")
    }

    /// Club-form words that sources put in front of or behind the same name ("CD Godoy Cruz", "Godoy Cruz", "1. FC Köln").
    static let noise: Set<String> = ["fc", "cf", "cd", "ca", "ac", "afc", "sc", "ud", "sd", "ad", "rcd", "ssc", "as", "fk", "sk", "bk", "club", "cp", "sv", "vfl", "vfb", "tsv", "fsv", "1"]

    static func squash(_ name: String) -> String {
        var tokens = fold(name).split(separator: " ", omittingEmptySubsequences: true).map(String.init)
        while tokens.count > 1, noise.contains(tokens[0]) { tokens.removeFirst() }
        while tokens.count > 1, noise.contains(tokens[tokens.count - 1]) { tokens.removeLast() }
        return tokens.joined(separator: " ")
    }

    /// Canonical name → the spellings the two sources (and Football-Data itself) use for the same club.
    static let aliases: [(canon: String, variants: [String])] = [
        ("man united", ["manchester united", "man utd", "manchester utd"]),
        ("man city", ["manchester city"]),
        ("tottenham", ["tottenham hotspur", "spurs"]),
        ("nottm forest", ["nottingham forest", "nott m forest", "nottm forest"]),
        ("wolves", ["wolverhampton", "wolverhampton wanderers"]),
        ("newcastle", ["newcastle united", "newcastle utd"]),
        ("west ham", ["west ham united"]),
        ("leeds", ["leeds united"]),
        ("brighton", ["brighton and hove albion", "brighton hove albion"]),
        ("sheffield united", ["sheffield utd"]),
        ("sheffield weds", ["sheffield wednesday"]),
        ("qpr", ["queens park rangers"]),
        ("west brom", ["west bromwich albion", "west bromwich"]),
        ("leicester", ["leicester city"]),
        ("norwich", ["norwich city"]),
        ("ath madrid", ["atletico madrid", "atletico de madrid", "atl madrid"]),
        ("ath bilbao", ["athletic bilbao", "athletic club", "athletic de bilbao"]),
        ("betis", ["real betis"]),
        ("sociedad", ["real sociedad"]),
        ("celta", ["celta vigo", "celta de vigo"]),
        ("espanol", ["espanyol", "rcd espanyol"]),
        ("vallecano", ["rayo vallecano"]),
        ("alaves", ["deportivo alaves"]),
        ("bayern munich", ["bayern munchen", "bayern"]),
        ("dortmund", ["borussia dortmund"]),
        ("m gladbach", ["mgladbach", "monchengladbach", "borussia monchengladbach", "gladbach"]),
        ("leverkusen", ["bayer leverkusen", "bayer 04 leverkusen"]),
        ("ein frankfurt", ["eintracht frankfurt", "frankfurt"]),
        ("rb leipzig", ["leipzig", "rasenballsport leipzig"]),
        ("stuttgart", ["vfb stuttgart"]),
        ("hoffenheim", ["tsg hoffenheim", "tsg 1899 hoffenheim"]),
        ("mainz", ["mainz 05", "fsv mainz 05"]),
        ("wolfsburg", ["vfl wolfsburg"]),
        ("werder bremen", ["bremen", "sv werder bremen"]),
        ("union berlin", ["1 fc union berlin"]),
        ("koln", ["fc koln", "cologne", "1 fc koln"]),
        ("st pauli", ["fc st pauli"]),
        ("inter", ["inter milan", "internazionale", "internazionale milano"]),
        ("milan", ["ac milan"]),
        ("napoli", ["ssc napoli"]),
        ("roma", ["as roma"]),
        ("lazio", ["ss lazio"]),
        ("verona", ["hellas verona"]),
        ("paris sg", ["psg", "paris saint germain", "paris saint-germain"]),
        ("marseille", ["olympique marseille", "olympique de marseille"]),
        ("lyon", ["olympique lyonnais", "olympique lyon"]),
        ("st etienne", ["saint etienne", "as saint etienne"]),
        ("monaco", ["as monaco"]),
        ("sp lisbon", ["sporting cp", "sporting lisbon", "sporting lisboa", "sporting clube de portugal", "sporting"]),
        ("sp braga", ["braga", "sporting braga", "sc braga"]),
        ("benfica", ["sl benfica"]),
        ("porto", ["fc porto"]),
        ("psv", ["psv eindhoven"]),
        ("ajax", ["ajax amsterdam"]),
        ("feyenoord", ["feyenoord rotterdam"]),
        ("club brugge", ["brugge", "club brugge kv"]),
        ("st truiden", ["sint truiden", "sint truidense"]),
        ("anderlecht", ["rsc anderlecht"]),
        ("standard", ["standard liege"]),
        ("rangers", ["glasgow rangers"]),
        ("hearts", ["heart of midlothian"]),
        ("river plate", ["ca river plate"]),
        ("boca juniors", ["ca boca juniors"]),
        ("ind rivadavia", ["independiente rivadavia"]),
        ("estudiantes l p", ["estudiantes la plata", "estudiantes de la plata"]),
        ("velez sarsfield", ["velez", "ca velez sarsfield"]),
        ("argentinos jrs", ["argentinos juniors"]),
        ("newells old boys", ["newells", "newell s old boys"]),
        ("gimnasia l p", ["gimnasia la plata", "gimnasia y esgrima la plata"]),
        ("talleres cordoba", ["talleres de cordoba", "talleres"]),
        ("central cordoba", ["central cordoba sde"]),
        ("union de santa fe", ["union santa fe", "union"]),
        ("belgrano", ["belgrano de cordoba", "belgrano cordoba"]),
        ("flamengo rj", ["flamengo"]),
        ("sao paulo", ["sao paulo fc"]),
        ("atletico mg", ["atletico mineiro"]),
        ("athletico pr", ["athletico paranaense", "atletico paranaense"]),
        ("botafogo rj", ["botafogo"]),
        ("vasco", ["vasco da gama"]),
        ("internacional", ["sc internacional"]),
        ("red bull bragantino", ["bragantino", "rb bragantino"]),
        ("santos", ["santos fc"]),
        ("sport recife", ["sport"]),
        ("america", ["club america", "america mexico"]),
        ("guadalajara", ["chivas", "guadalajara chivas", "chivas guadalajara"]),
        ("pumas unam", ["unam", "pumas", "unam pumas"]),
        ("tigres uanl", ["tigres"]),
        ("monterrey", ["cf monterrey"]),
        ("leon", ["club leon"]),
        ("tijuana", ["club tijuana", "xolos"]),
        ("la galaxy", ["los angeles galaxy", "l a galaxy"]),
        ("los angeles fc", ["lafc"]),
        ("new york city", ["nycfc", "new york city fc"]),
        ("new york red bulls", ["ny red bulls"]),
        ("inter miami", ["inter miami cf"]),
    ]

    private static let aliasMap: [String: String] = {
        var map: [String: String] = [:]
        for entry in aliases {
            let canon = FreeStats.squash(entry.canon)
            for variant in entry.variants { map[FreeStats.squash(variant)] = canon }
            map[canon] = canon
        }
        return map
    }()

    /// The key two spellings of one club share. Exact equality of these keys is the only way two names match.
    static func teamKey(_ name: String) -> String {
        let s = squash(name)
        return aliasMap[s] ?? s
    }

    struct FootballIndex {
        private var teams: [String: [TeamForm]] = [:]

        init(_ sources: [[TeamForm]] = []) {
            for list in sources { for t in list { teams[FreeStats.teamKey(t.name), default: []].append(t) } }
        }

        /// Found, or the reason there is no data. Two different files with the same key are "ambiguous", not a guess.
        func lookup(_ name: String) throws -> TeamForm {
            let found = teams[FreeStats.teamKey(name)] ?? []
            if found.isEmpty { throw Miss(reason: "no está en las ligas de Football-Data") }
            if found.count == 1 { return found[0] }
            throw Miss(reason: "nombre ambiguo (\(found.count) equipos con ese nombre)")
        }
    }

    /// Women's, youth and reserve competitions share club names with the men's first team: never matched.
    static func notSeniorMen(_ tournament: String) -> Bool {
        let t = fold(tournament)
        let words: Set<String> = ["women", "womens", "femenino", "femenina", "feminino", "feminin", "reserve", "reserves", "reserva", "reservas", "youth", "juvenil", "u17", "u18", "u19", "u20", "u21", "u23"]
        return t.split(separator: " ").contains { words.contains(String($0)) }
            || t.contains("sub 20") || t.contains("sub 23") || t.contains("sub 17") || t.contains("sub 19")
    }

    // MARK: - Tennis

    /// One finished singles match, compact for the cache (serialised as an array; see the Codable extension below).
    struct TMatch: Equatable, Sendable {
        /// tourney_date (start of the tournament) as days since the epoch
        var day: Int
        /// "atp", "challenger" or "wta"
        var tour: String
        var tourney: String
        var surface: String
        var round: String
        var winnerID: String
        var winnerName: String
        var loserID: String
        var loserName: String
        var winnerRank: String
        var loserRank: String
        /// tourney_id + match number, to drop a match seen in the year file and in the ongoing file
        var key: String
    }

    static func roundOrder(_ round: String) -> Int {
        switch round {
        case "R128": return 10
        case "R64": return 20
        case "R32": return 30
        case "R16": return 40
        case "QF": return 50
        case "SF": return 60
        case "BR": return 65
        case "F": return 70
        case "RR": return 5
        default: return round.hasPrefix("Q") ? 1 : 0
        }
    }

    /// Played singles matches of a TennisMyLife file. Walkovers are skipped (nobody played); retirements count.
    static func parseTennis(_ text: String, tour: String) throws -> [TMatch] {
        let table = Table(text)
        guard table.has(["tourney_name", "surface", "tourney_date", "winner_id", "winner_name", "loser_id", "loser_name", "score", "round"]) else {
            throw MessageError("TennisMyLife: el archivo no tiene las columnas esperadas.")
        }
        var out: [TMatch] = []
        for row in table.rows {
            guard let day = parseDate(table.get(row, "tourney_date")) else { continue }
            let wid = table.get(row, "winner_id")
            let lid = table.get(row, "loser_id")
            let score = table.get(row, "score")
            if wid.isEmpty || lid.isEmpty || score.isEmpty || score.contains("W/O") || score.contains("DEF") { continue }
            let key = "\(table.get(row, "tourney_id"))#\(table.get(row, "match_num"))#\(wid)#\(lid)"
            out.append(TMatch(day: day, tour: tour, tourney: table.get(row, "tourney_name"), surface: table.get(row, "surface"),
                              round: table.get(row, "round"), winnerID: wid, winnerName: table.get(row, "winner_name"),
                              loserID: lid, loserName: table.get(row, "loser_name"),
                              winnerRank: table.get(row, "winner_rank"), loserRank: table.get(row, "loser_rank"), key: key))
        }
        return out
    }

    struct Surface: Equatable, Sendable {
        var name: String
        var wins: Int
        var losses: Int
    }

    struct PlayerForm: Equatable, Sendable {
        var id = ""
        var name = ""
        var tour = ""
        /// Last 5 results, oldest to newest.
        var form5 = ""
        /// This year's wins and losses by surface.
        var surfaces: [Surface] = []
        var lastTourney = ""
        var lastRound = ""
        var lastWon = false
        /// Start date of the last tournament (ISO); TennisMyLife dates a match by its tournament.
        var last = ""
        var rank = ""
        var tokens: [String] = []
    }

    static func nameTokens(_ name: String) -> [String] {
        let cleaned = name.replacingOccurrences(of: ".", with: " ").replacingOccurrences(of: "-", with: " ")
        return fold(cleaned).split(separator: " ", omittingEmptySubsequences: true).map(String.init)
    }

    /// "Surname I." / "Surname I J" (initials) or "First Surname" (any order of the same words) against a full name.
    static func tennisNameMatches(_ fixture: [String], _ player: [String]) -> Bool {
        if fixture.count < 2 || player.count < 2 { return false }
        var k = fixture.count
        while k > 0, fixture[k - 1].count == 1 { k -= 1 }
        if k < fixture.count {
            if k == 0 { return false }
            let surname = Array(fixture[0..<k])
            let initials = Array(fixture[k...])
            for s in 1..<player.count {
                guard Array(player[s...]) == surname else { continue }
                let given: [Character] = player[0..<s].compactMap { $0.first }
                if given.count >= initials.count && zip(initials, given).allSatisfy({ $0.first == $1 }) { return true }
            }
            return false
        }
        return fixture.sorted() == player.sorted()
    }

    struct TennisIndex {
        var players: [PlayerForm] = []

        /// `year` is the current year (surface record "this year").
        init(matches: [TMatch] = [], year: Int = 0) {
            let sorted = matches.sorted { a, b in
                if a.day != b.day { return a.day < b.day }
                let (ra, rb) = (FreeStats.roundOrder(a.round), FreeStats.roundOrder(b.round))
                if ra != rb { return ra < rb }
                return a.key < b.key
            }
            var all: [TMatch] = []
            for m in sorted {
                if let prev = all.last, prev.key == m.key { continue }
                all.append(m)
            }
            var byID: [String: [(m: TMatch, won: Bool)]] = [:]
            for m in all {
                byID[m.winnerID, default: []].append((m: m, won: true))
                byID[m.loserID, default: []].append((m: m, won: false))
            }
            var out: [PlayerForm] = []
            for id in byID.keys.sorted() {
                guard let games = byID[id], let lastGame = games.last else { continue }
                let last = lastGame.m
                let tail = games.suffix(5)
                var wins: [String: Int] = [:]
                var losses: [String: Int] = [:]
                for g in games {
                    if FreeStats.civilFromDays(g.m.day).year != year || g.m.surface.isEmpty { continue }
                    if g.won { wins[g.m.surface, default: 0] += 1 } else { losses[g.m.surface, default: 0] += 1 }
                }
                let names = Set(wins.keys).union(losses.keys).sorted()
                let name = lastGame.won ? last.winnerName : last.loserName
                out.append(PlayerForm(
                    id: id, name: name, tour: last.tour,
                    form5: String(tail.map { $0.won ? Character("W") : Character("L") }),
                    surfaces: names.map { Surface(name: $0, wins: wins[$0] ?? 0, losses: losses[$0] ?? 0) },
                    lastTourney: last.tourney, lastRound: last.round, lastWon: lastGame.won, last: FreeStats.iso(last.day),
                    rank: lastGame.won ? last.winnerRank : last.loserRank,
                    tokens: FreeStats.nameTokens(name)))
            }
            players = out
        }

        /// `wta`: true when the tournament says women's tour, false for ATP/Challenger, nil when unknown.
        func lookup(_ name: String, wta: Bool?) throws -> PlayerForm {
            let tokens = FreeStats.nameTokens(name)
            let found = players.filter { p in
                let tourOK: Bool
                if let wta { tourOK = (p.tour == "wta") == wta } else { tourOK = true }
                return tourOK && FreeStats.tennisNameMatches(tokens, p.tokens)
            }
            if found.isEmpty { throw Miss(reason: "no está en los archivos ATP/Challenger/WTA de TennisMyLife") }
            if found.count == 1 { return found[0] }
            throw Miss(reason: "nombre ambiguo (\(found.count) jugadores)")
        }
    }

    static func tourHint(_ tournament: String) -> Bool? {
        let words = Set(fold(tournament).split(separator: " ").map(String.init))
        if !words.isDisjoint(with: ["wta", "femenino", "femenina", "women", "womens"]) { return true }
        if !words.isDisjoint(with: ["atp", "challenger", "masculino"]) { return false }
        return nil
    }

    // MARK: - Fixtures, rows, text and files

    /// A match of the day, as the morning snapshot saved it.
    struct Fixture: Equatable, Sendable, Codable {
        var id: String
        var home: String
        var away: String
        var tournament: String
        var sport: String
        /// Unix seconds.
        var start: Int64
    }

    struct DailyFixtures: Codable, Equatable, Sendable {
        var day: String
        var fixtures: [Fixture]
    }

    /// `odds/daily-fixtures.json`: the day's match list for the free stats. Odds.swift's morning snapshot only lists
    /// football (so far), hence the fixed `sport`.
    static func fixturesJSON(day: String, fixtures: [Odds.Fixture]) -> Data {
        let list = fixtures.map { Fixture(id: $0.id, home: $0.home, away: $0.away, tournament: $0.tournament, sport: "Fútbol", start: $0.start) }
        return (try? JSONEncoder().encode(DailyFixtures(day: day, fixtures: list))) ?? Data("{}".utf8)
    }

    /// Today's fixtures (nil when the file is missing, broken or from another day).
    static func loadDailyFixtures(day: String, cacheDir: URL) -> [Fixture]? {
        guard let data = try? Data(contentsOf: cacheDir.appendingPathComponent("daily-fixtures.json")),
              let parsed = try? JSONDecoder().decode(DailyFixtures.self, from: data), parsed.day == day else { return nil }
        return parsed.fixtures
    }

    /// One side of one fixture: the form found or the reason there is none.
    struct Row: Equatable, Sendable {
        var fixtureID = ""
        var sport = ""
        var tournament = ""
        var start = ""
        /// "local" or "visita" (tennis: first and second player as the provider lists them).
        var side = ""
        var name = ""
        var found = false
        var reason = ""
        var matched = ""
        var source = ""
        var dataDay = ""
        var lastMatch = ""
        var form5 = ""
        var gf5 = ""
        var ga5 = ""
        var n5 = ""
        var cond = ""
        var condForm = ""
        var condPts = ""
        var condN = ""
        var ppg = ""
        var played = ""
        var surfaces = ""
        var lastTourney = ""
        var rank = ""
        /// The fragment of the "Forma:" line.
        var text = ""
    }

    static func isFootball(_ sport: String) -> Bool { let s = fold(sport); return s.contains("futbol") || s.contains("soccer") || s == "football" }
    static func isTennis(_ sport: String) -> Bool { let s = fold(sport); return s.contains("tenis") || s.contains("tennis") }
    static func isDoubles(_ name: String) -> Bool { name.contains("/") }

    private static func sinDatos(_ f: Fixture, _ side: String, _ name: String, _ reason: String) -> Row {
        Row(fixtureID: f.id, sport: f.sport, tournament: f.tournament, start: Odds.isoUTC(f.start), side: side, name: name,
            reason: reason, text: "\(name) sin datos (\(reason))")
    }

    private static func fixed2(_ value: Double) -> String { String(format: "%.2f", value) }

    private static func footballRow(_ f: Fixture, home: Bool, index: FootballIndex, dataDay: String) -> Row {
        let side = home ? "local" : "visita"
        let name = home ? f.home : f.away
        if notSeniorMen(f.tournament) {
            return sinDatos(f, side, name, "torneo femenino, juvenil o de reservas: Football-Data trae solo primeros equipos masculinos")
        }
        let t: TeamForm
        do { t = try index.lookup(name) } catch { return sinDatos(f, side, name, (error as? Miss)?.reason ?? "sin datos") }
        let cond = home ? t.home : t.away
        var text = "\(name) \(t.form5) (\(t.gf5) GF/\(t.ga5) GC en \(t.n5) PJ"
        if cond.n > 0 { text += "; \(side) \(cond.pts) pts en \(cond.n) PJ" }
        text += "; \(fixed2(t.ppg)) pts/PJ en \(t.played) PJ de la temporada)"
        return Row(fixtureID: f.id, sport: f.sport, tournament: f.tournament, start: Odds.isoUTC(f.start), side: side, name: name,
                   found: true, matched: t.name, source: "Football-Data \(t.league)", dataDay: dataDay, lastMatch: t.last,
                   form5: t.form5, gf5: String(t.gf5), ga5: String(t.ga5), n5: String(t.n5),
                   cond: side, condForm: cond.form, condPts: String(cond.pts), condN: String(cond.n),
                   ppg: fixed2(t.ppg), played: String(t.played), text: text)
    }

    private static func tennisRow(_ f: Fixture, first: Bool, index: TennisIndex, dataDay: String) -> Row {
        let side = first ? "local" : "visita"
        let name = first ? f.home : f.away
        if isDoubles(name) { return sinDatos(f, side, name, "dobles: no se calcula forma de parejas") }
        let p: PlayerForm
        do { p = try index.lookup(name, wta: tourHint(f.tournament)) } catch { return sinDatos(f, side, name, (error as? Miss)?.reason ?? "sin datos") }
        let surfaces = p.surfaces.map { "\($0.name) \($0.wins)-\($0.losses)" }.joined(separator: ", ")
        let outcome = p.lastWon ? "ganó" : "perdió"
        let last = "\(p.lastTourney) (\(dmy(parseDate(p.last) ?? 0)), \(p.lastRound), \(outcome))"
        var parts: [String] = []
        if !surfaces.isEmpty { parts.append("este año \(surfaces)") }
        parts.append("último torneo \(last)")
        if !p.rank.isEmpty { parts.append("ranking \(p.rank)") }
        let text = "\(name) \(p.form5) (\(parts.joined(separator: "; ")))"
        return Row(fixtureID: f.id, sport: f.sport, tournament: f.tournament, start: Odds.isoUTC(f.start), side: side, name: name,
                   found: true, matched: p.name, source: "TennisMyLife", dataDay: dataDay, lastMatch: p.last,
                   form5: p.form5, n5: String(p.form5.count), surfaces: surfaces, lastTourney: last, rank: p.rank, text: text)
    }

    /// Two rows per football fixture and per tennis singles fixture, in fixture order. Other sports have no free source here.
    static func rowsFor(_ fixtures: [Fixture], football: FootballIndex, tennis: TennisIndex, dataDay: String) -> [Row] {
        var rows: [Row] = []
        for f in fixtures {
            if isFootball(f.sport) {
                rows.append(footballRow(f, home: true, index: football, dataDay: dataDay))
                rows.append(footballRow(f, home: false, index: football, dataDay: dataDay))
            } else if isTennis(f.sport) {
                rows.append(tennisRow(f, first: true, index: tennis, dataDay: dataDay))
                rows.append(tennisRow(f, first: false, index: tennis, dataDay: dataDay))
            }
        }
        return rows
    }

    static let formaHeading = "## Forma reciente"

    /// The "Forma reciente" block for `resumen-del-dia.md`: one `Forma:` line per match, with the source and data dates.
    static func formaSection(_ rows: [Row], day: String) -> String {
        var out = "\(formaHeading) (Football-Data.co.uk y TennisMyLife, descargado el \(day))\n\n"
            + "Cada línea trae los últimos 5 resultados (el último carácter es el partido más reciente; W ganó, D empató, L perdió), goles a favor/en contra de esos 5, la forma de local o de visita y los puntos por partido de la temporada. "
            + "«sin datos» significa que el equipo o jugador no está en esas fuentes: no lo supongas, búscalo en la web. stats-hoy.csv trae las mismas cifras en columnas. Cita Football-Data o TennisMyLife como fuente de estas cifras.\n\n"
        var i = 0
        while i < rows.count {
            let a = rows[i]
            let next: Row? = (i + 1 < rows.count && rows[i + 1].fixtureID == a.fixtureID) ? rows[i + 1] : nil
            i += next != nil ? 2 : 1
            guard let b = next else { continue }
            var label = ""
            if let firstFound = [a, b].first(where: { $0.found }) {
                label = firstFound.source.hasPrefix("Football-Data") ? "Football-Data" : "TennisMyLife"
            }
            var line = "- \(a.tournament): \(a.name) vs \(b.name). Forma: \(a.text) · \(b.text)"
            if !label.isEmpty {
                let dates = [a, b].filter { $0.found }.compactMap { parseDate($0.lastMatch) }.map(dmy)
                line += ", fuente \(label) (descargado \(day); último partido registrado \(dates.joined(separator: " y ")))"
            }
            out += line + "\n"
        }
        return out
    }

    /// Replaces (or adds) the "Forma reciente" block at the end of the summary; running twice never duplicates it.
    static func applyToSummary(_ md: String, section: String) -> String {
        var base = md
        if let range = md.range(of: "\n" + formaHeading) { base = String(md[md.startIndex..<range.lowerBound]) }
        while let last = base.last, last.isWhitespace { base.removeLast() }
        return base + "\n\n" + section
    }

    /// Same cell rule as the Windows CSV: a formula-looking cell gets a leading apostrophe.
    static func csvCell(_ s: String) -> String {
        let trimmed = String(s.drop(while: { $0.isWhitespace }))
        let formula = ["=", "+", "-", "@"].contains { trimmed.hasPrefix($0) } || ["\t", "\r", "\n"].contains { s.hasPrefix($0) }
        return "\"" + (formula ? "'" : "") + s.replacingOccurrences(of: "\"", with: "\"\"") + "\""
    }

    static let csvHeader = "fixtureId,deporte,torneo,inicio_utc,lado,nombre,estado,motivo,coincide_con,fuente,fecha_descarga,ultimo_partido,forma5,gf5,gc5,pj5,condicion,forma_condicion,pts_condicion,pj_condicion,ppg_temporada,pj_temporada,superficie_ano,ultimo_torneo,ranking\n"

    static func statsCSV(_ rows: [Row]) -> String {
        var out = csvHeader
        for r in rows {
            let state: String = r.found ? "ok" : "sin datos"
            var cells: [String] = [r.fixtureID, r.sport, r.tournament, r.start, r.side, r.name, state, r.reason, r.matched, r.source]
            cells += [r.dataDay, r.lastMatch, r.form5, r.gf5, r.ga5, r.n5, r.cond, r.condForm, r.condPts, r.condN]
            cells += [r.ppg, r.played, r.surfaces, r.lastTourney, r.rank]
            out += cells.map { csvCell($0) }.joined(separator: ",") + "\n"
        }
        return out
    }

    // MARK: - Cache: one request per file per day

    struct Source: Codable, Equatable, Sendable {
        /// Local day (ISO) the file was last asked for; it is not asked for again that day.
        var day = ""
        var etag = ""
        var modified = ""
        /// "ok" or "missing" (404).
        var status = ""
    }

    struct Cache: Codable, Equatable, Sendable {
        var sources: [String: Source] = [:]
        var football: [String: [TeamForm]] = [:]
        var tennis: [String: [TMatch]] = [:]
        /// Day (ISO) every source needed for that day's fixtures was reached.
        var doneDay = ""

        init() {}

        init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            sources = (try? c.decodeIfPresent([String: Source].self, forKey: .sources)) ?? [:]
            football = (try? c.decodeIfPresent([String: [TeamForm]].self, forKey: .football)) ?? [:]
            tennis = (try? c.decodeIfPresent([String: [TMatch]].self, forKey: .tennis)) ?? [:]
            doneDay = (try? c.decodeIfPresent(String.self, forKey: .doneDay)) ?? ""
        }
    }

    /// A file is requested only when it has not been asked for today.
    static func needsFetch(_ cache: Cache, key: String, day: String) -> Bool { cache.sources[key]?.day != day }

    static func mark(_ cache: inout Cache, key: String, day: String, status: String, etag: String = "", modified: String = "") {
        cache.sources[key] = Source(day: day, etag: etag, modified: modified, status: status)
    }

    static func cacheURL(_ cacheDir: URL) -> URL { cacheDir.appendingPathComponent("stats-cache.json") }

    static func loadCache(_ cacheDir: URL) -> Cache {
        (try? Data(contentsOf: cacheURL(cacheDir))).flatMap { try? JSONDecoder().decode(Cache.self, from: $0) } ?? Cache()
    }

    static func saveCache(_ cache: Cache, _ cacheDir: URL) {
        try? FileManager.default.createDirectory(at: cacheDir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        if let data = try? JSONEncoder().encode(cache) { try? data.write(to: cacheURL(cacheDir), options: .atomic) }
    }

    /// Drops cached files that can no longer be used (an old season, an old year).
    static func prune(_ cache: inout Cache, season: String, year: Int, month: Int) {
        let prev = previousSeason(season)
        func keepFootball(_ k: String) -> Bool {
            guard k.hasPrefix("fd:") else { return true }
            let parts = k.components(separatedBy: ":")
            return parts.count < 3 || parts[2] == season || parts[2] == prev
        }
        let usable: [String] = month <= 2 ? [String(year), String(year - 1)] : [String(year)]
        func keepTennis(_ k: String) -> Bool {
            !k.hasPrefix("tml:") || k.contains("ongoing") || usable.contains { k.hasPrefix("tml:" + $0) }
        }
        cache.sources = cache.sources.filter { keepFootball($0.key) && keepTennis($0.key) }
        cache.football = cache.football.filter { keepFootball($0.key) }
        cache.tennis = cache.tennis.filter { keepTennis($0.key) }
    }

    // MARK: - Network (the only part that talks to the internet)

    enum Got: Sendable {
        case body(text: String, etag: String, modified: String)
        case notModified
        case missing
    }

    static func decode(_ data: Data) -> String {
        String(data: data, encoding: .utf8) ?? String(data: data, encoding: .isoLatin1) ?? ""
    }

    /// A polite client: identified, redirects followed, a pause between requests, size capped.
    actor Net {
        private var last: Date?
        private let session: URLSession

        init() {
            let config = URLSessionConfiguration.ephemeral
            config.timeoutIntervalForRequest = 60
            config.timeoutIntervalForResource = 120
            config.requestCachePolicy = .reloadIgnoringLocalCacheData
            session = URLSession(configuration: config)
        }

        func get(_ urlText: String, previous: Source?) async throws -> Got {
            if let last {
                let wait = Double(FreeStats.gapMs) / 1000 - Date().timeIntervalSince(last)
                if wait > 0 { try? await Task.sleep(for: .milliseconds(Int(wait * 1000))) }
            }
            last = Date()
            guard let url = URL(string: urlText) else { throw MessageError("No se pudo preparar la conexión.") }
            let host = url.host ?? "la fuente"
            var request = URLRequest(url: url, timeoutInterval: 60)
            request.setValue(FreeStats.userAgent, forHTTPHeaderField: "User-Agent")
            request.setValue("text/csv,*/*", forHTTPHeaderField: "Accept")
            if let previous {
                if !previous.etag.isEmpty { request.setValue(previous.etag, forHTTPHeaderField: "If-None-Match") }
                if !previous.modified.isEmpty { request.setValue(previous.modified, forHTTPHeaderField: "If-Modified-Since") }
            }
            let result: (Data, URLResponse)
            do { result = try await session.data(for: request) } catch { throw MessageError("Sin conexión con \(host).") }
            let (data, response) = result
            guard let http = response as? HTTPURLResponse else { throw MessageError("\(host) no respondió.") }
            switch http.statusCode {
            case 304: return .notModified
            case 404: return .missing
            case 200:
                if data.count > FreeStats.maxBytes { throw MessageError("El archivo es más grande de lo esperado.") }
                return .body(text: FreeStats.decode(data), etag: http.value(forHTTPHeaderField: "ETag") ?? "",
                             modified: http.value(forHTTPHeaderField: "Last-Modified") ?? "")
            default: throw MessageError("\(host) respondió \(http.statusCode).")
            }
        }
    }

    /// Refreshes one football file unless it was asked for today. True when the cache has data for it afterwards.
    static func refreshFootball(_ net: Net, _ cache: inout Cache, key: String, url: String, league: String, day: String) async throws -> Bool {
        if !needsFetch(cache, key: key, day: day) { return cache.football[key] != nil }
        let previous: Source? = cache.football[key] != nil ? cache.sources[key] : nil
        switch try await net.get(url, previous: previous) {
        case .notModified:
            let s = cache.sources[key] ?? Source()
            mark(&cache, key: key, day: day, status: "ok", etag: s.etag, modified: s.modified)
            return true
        case .missing:
            cache.football[key] = nil
            mark(&cache, key: key, day: day, status: "missing")
            return false
        case .body(let text, let etag, let modified):
            let matches = try parseFootball(text, defaultLeague: league)
            if matches.isEmpty {
                cache.football[key] = nil
                mark(&cache, key: key, day: day, status: "missing")
                return false
            }
            cache.football[key] = footballForms(matches, source: key)
            mark(&cache, key: key, day: day, status: "ok", etag: etag, modified: modified)
            return true
        }
    }

    static func refreshTennis(_ net: Net, _ cache: inout Cache, key: String, file: String, tour: String, day: String) async throws {
        if !needsFetch(cache, key: key, day: day) { return }
        let previous: Source? = cache.tennis[key] != nil ? cache.sources[key] : nil
        switch try await net.get("\(tmlBase)/\(file).csv", previous: previous) {
        case .notModified:
            let s = cache.sources[key] ?? Source()
            mark(&cache, key: key, day: day, status: "ok", etag: s.etag, modified: s.modified)
        case .missing:
            // No ongoing tournaments (or no file yet this year): nothing to add.
            cache.tennis[key] = nil
            mark(&cache, key: key, day: day, status: "missing")
        case .body(let text, let etag, let modified):
            cache.tennis[key] = try parseTennis(text, tour: tour)
            mark(&cache, key: key, day: day, status: "ok", etag: etag, modified: modified)
        }
    }

    static func mainKey(_ code: String, _ season: String) -> String { "fd:\(code):\(season)" }

    /// Is some football team of the day still without data (not "ambiguous": that is a different answer)?
    private static func anyUnmatched(_ wants: [Fixture], cache: Cache, fresh: [String]) -> Bool {
        let index = FootballIndex(fresh.compactMap { cache.football[$0] })
        for f in wants {
            for name in [f.home, f.away] {
                do { _ = try index.lookup(name) } catch {
                    if let miss = error as? Miss, miss.reason.hasPrefix("no está") { return true }
                }
            }
        }
        return false
    }

    /// All the downloads of a day and the rows built from them. Returns the rows and whether every file was reached
    /// (otherwise it is asked for again later; the files already reached today are not).
    static func collect(_ fixtures: [Fixture], cache: inout Cache, now: Date, timeZone: TimeZone) async -> (rows: [Row], complete: Bool) {
        let local = Int(now.timeIntervalSince1970.rounded(.down)) + timeZone.secondsFromGMT(for: now)
        let today = Int((Double(local) / 86_400).rounded(.down))
        let (year, month, _) = civilFromDays(today)
        let day = iso(today)
        let season = seasonCode(year: year, month: month)
        prune(&cache, season: season, year: year, month: month)
        let net = Net()
        var complete = true

        let wantsFootball = fixtures.filter { isFootball($0.sport) && !notSeniorMen($0.tournament) }
        let wantsTennis = fixtures.contains { isTennis($0.sport) && (!isDoubles($0.home) || !isDoubles($0.away)) }

        var fresh: [String] = []
        if !wantsFootball.isEmpty {
            for league in mainLeagues {
                let key = mainKey(league.code, season)
                do {
                    let reached = try await refreshFootball(net, &cache, key: key, url: "\(fdBase)/mmz4281/\(season)/\(league.code).csv", league: league.name, day: day)
                    if reached {
                        fresh.append(key)
                    } else {
                        // Early in a season the new folder may not exist yet: use the previous one.
                        let pseason = previousSeason(season)
                        let pkey = mainKey(league.code, pseason)
                        do {
                            let again = try await refreshFootball(net, &cache, key: pkey, url: "\(fdBase)/mmz4281/\(pseason)/\(league.code).csv", league: league.name, day: day)
                            if again { fresh.append(pkey) }
                        } catch {
                            complete = false
                            MikaLog.info("stats: \(league.code): \(MessageError.text(error))")
                        }
                    }
                } catch {
                    complete = false
                    MikaLog.info("stats: \(league.code): \(MessageError.text(error))")
                }
            }
            if anyUnmatched(wantsFootball, cache: cache, fresh: fresh) {
                for country in extraCountries {
                    let key = "fd:\(country)"
                    do {
                        let reached = try await refreshFootball(net, &cache, key: key, url: "\(fdBase)/new/\(country).csv", league: country, day: day)
                        if reached { fresh.append(key) }
                    } catch {
                        complete = false
                        MikaLog.info("stats: \(country): \(MessageError.text(error))")
                    }
                    if !anyUnmatched(wantsFootball, cache: cache, fresh: fresh) { break }
                }
            }
        }
        if wantsTennis {
            var files: [(key: String, file: String, tour: String)] = []
            let years = month <= 2 ? [year, year - 1] : [year]
            for y in years {
                files.append((key: "tml:\(y)", file: "\(y)", tour: "atp"))
                files.append((key: "tml:\(y)_challenger", file: "\(y)_challenger", tour: "challenger"))
                files.append((key: "tml:\(y)_wta", file: "\(y)_wta", tour: "wta"))
            }
            files.append((key: "tml:ongoing_tourneys", file: "ongoing_tourneys", tour: "atp"))
            files.append((key: "tml:challenger_ongoing_tourneys", file: "challenger_ongoing_tourneys", tour: "challenger"))
            files.append((key: "tml:wta_ongoing_tourneys", file: "wta_ongoing_tourneys", tour: "wta"))
            for f in files {
                do {
                    try await refreshTennis(net, &cache, key: f.key, file: f.file, tour: f.tour, day: day)
                } catch {
                    complete = false
                    MikaLog.info("stats: \(f.file): \(MessageError.text(error))")
                }
            }
        }

        let football = FootballIndex(fresh.compactMap { cache.football[$0] })
        var allTennis: [TMatch] = []
        for (key, list) in cache.tennis where cache.sources[key]?.day == day { allTennis += list }
        let tennis = TennisIndex(matches: allTennis, year: year)
        return (rowsFor(fixtures, football: football, tennis: tennis, dataDay: day), complete)
    }

    /// `stats-hoy.csv` and the "Forma reciente" section of `resumen-del-dia.md`, in every directory.
    static func writeOutputs(_ rows: [Row], day: String, dirs: [URL]) {
        let csv = "\u{FEFF}" + statsCSV(rows)
        let section = formaSection(rows, day: day)
        for dir in dirs {
            try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            try? Data(csv.utf8).write(to: dir.appendingPathComponent("stats-hoy.csv"), options: .atomic)
            let summary = dir.appendingPathComponent("resumen-del-dia.md")
            if let md = try? String(contentsOf: summary, encoding: .utf8) {
                try? Data(applyToSummary(md, section: section).utf8).write(to: summary, options: .atomic)
            }
        }
    }

    // MARK: - The daily step

    enum Step: Equatable, Sendable {
        /// No fixture list for today yet, or a retry is not due.
        case waiting
        /// Today's work is finished.
        case done
        /// A run was made; `complete` when every file was reached.
        case attempted(complete: Bool)
    }

    /// Called every minute by the morning loop once `useFreeStats` is on. `mayAttempt` is false within `retry` of a failed
    /// attempt. `cacheDir` is `MIKA/odds/` (where `daily-fixtures.json` and `stats-cache.json` live); `rawDirs` are
    /// PARLEY's workspace `odds/` folders. Twin of `daily_step` in stats.rs.
    static func dailyStep(now: Date, timeZone: TimeZone, cacheDir: URL, rawDirs: [URL], mayAttempt: Bool) async -> Step {
        let day = ChatArchive.dateLabel(now, timeZone: timeZone)
        guard let fixtures = loadDailyFixtures(day: day, cacheDir: cacheDir) else { return .waiting }
        var cache = loadCache(cacheDir)
        if cache.doneDay == day { return .done }
        if !mayAttempt { return .waiting }
        let result = await collect(fixtures, cache: &cache, now: now, timeZone: timeZone)
        writeOutputs(result.rows, day: day, dirs: [cacheDir] + rawDirs)
        if result.complete { cache.doneDay = day }
        saveCache(cache, cacheDir)
        return .attempted(complete: result.complete)
    }
}

extension FreeStats.TMatch: Codable {
    init(from decoder: Decoder) throws {
        var c = try decoder.unkeyedContainer()
        day = try c.decode(Int.self)
        tour = try c.decode(String.self)
        tourney = try c.decode(String.self)
        surface = try c.decode(String.self)
        round = try c.decode(String.self)
        winnerID = try c.decode(String.self)
        winnerName = try c.decode(String.self)
        loserID = try c.decode(String.self)
        loserName = try c.decode(String.self)
        winnerRank = try c.decode(String.self)
        loserRank = try c.decode(String.self)
        key = try c.decode(String.self)
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.unkeyedContainer()
        try c.encode(day)
        for text in [tour, tourney, surface, round, winnerID, winnerName, loserID, loserName, winnerRank, loserRank, key] { try c.encode(text) }
    }
}
