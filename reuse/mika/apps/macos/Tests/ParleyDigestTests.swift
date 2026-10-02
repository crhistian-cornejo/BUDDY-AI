import XCTest

final class ParleyDigestTests: XCTestCase {
    static let sample = """
    ```json
    {"resumen":"Día con valor en goles","metodo":"Poisson y forma",
     "partidos":[
      {"deporte":"futbol","torneo":"Clasificación europea sub-21","local":"Austria","visita":"Dinamarca","hora":"11:00","cuotas":"1 @2.72 · X @3.25 · 2 @2.40",
       "datos":{"gf_local":"1,8","gc_local":0.9,"gf_visita":1.4,"gc_visita":1.2,"forma_local":"G-E-P-G-G","forma_visita":"DLWWW","h2h":"1-3-0"},
       "estadisticas":["Austria 9 GF en 5"],"bajas":"Ninguna confirmada","alineaciones":"probables",
       "mercados":[{"mercado":"Gana Austria","prob":"45%","cuota":2.72,"nota":"local fuerte"},{"mercado":"Más de 2.5 goles","prob":0.55,"cuota":null},
                   {"mercado":"Ambos marcan","prob":0.52},{"mercado":"Córners más de 9.5","prob":0.5}],
       "pick":{"mercado":"Más de 2.5 goles","prob":0.55,"cuota":null,"confianza":"Media","motivo":"ambos anotan"},
       "fuentes":["https://uefa.com/x"]},
      {"deporte":"Tenis","torneo":"ATP Beijing","partido":"Sinner vs Rune","hora":"05:00","estadisticas":["<b>Rune</b> 60% en dura"],
       "datos":{"ranking_local":1,"ranking_visita":12,"pct_superficie_local":0.85,"pct_superficie_visita":"60"},
       "mercados":[{"mercado":"Gana Sinner","prob":0.78,"cuota":1.25}],"pick":{"mercado":"Gana Sinner","prob":0.78,"cuota":1.25,"confianza":"alta","motivo":"x"}},
      {"deporte":"futbol","partido":""}
     ],
     "picks":[{"partido":"Austria vs Dinamarca","mercado":"Más de 2.5 goles","prob":0.55,"cuota":null,"monto":"S/ 4","confianza":"media","motivo":"m","fuente":"https://uefa.com"},
              {"partido":"Sinner vs Rune","mercado":"Gana Sinner","prob":0.78,"cuota":"1,25","confianza":"alta"}],
     "combinadas":[{"selecciones":["Austria gana @2.72","Sinner gana @1.25"],"prob":0.35,"cuota":3.4,"monto":"S/ 2"},{"selecciones":["solo una"]}],
     "telegram":[{"canal":"JC","autor":"Juan","hora":"09:10","pick":"Bélgica más de 2.5","cuota":1.54,"evidencia":"Captura","prob":0.6,"veredicto":"A favor","motivo":"m"},
                 {"canal":"Neo","pick":"Builder oculto","veredicto":"en contra"}],
     "ojo":["rotaciones"]}
    ```
    """

    func testParsesTheAnalystSchemaTolerantly() throws {
        let d = try XCTUnwrap(ParleyDigest.parse(Self.sample, day: "2026-10-02", generated: "07:40"))
        XCTAssertEqual(d.matches.count, 2, "a row without a match name is dropped")
        let a = d.matches[0]
        XCTAssertEqual([a.home, a.away, a.match], ["Austria", "Dinamarca", "Austria vs Dinamarca"])
        XCTAssertEqual(a.goalsForHome, 1.8)
        XCTAssertEqual(a.formHome, "WDLWW", "G/E/P become W/D/L")
        XCTAssertEqual(a.markets.count, 4)
        XCTAssertEqual(a.markets[0].prob ?? 0, 0.45, accuracy: 1e-9)
        XCTAssertNil(a.markets[1].odds)
        XCTAssertEqual(a.pick?.confidence, "media")
        let t = d.matches[1]
        XCTAssertEqual(t.sport, "tenis")
        XCTAssertEqual([t.home, t.away], ["Sinner", "Rune"])
        XCTAssertEqual(t.surfaceAway, 60)
        XCTAssertEqual(d.picks.count, 2)
        XCTAssertEqual(d.picks[1].odds, 1.25)
        XCTAssertEqual(d.combos.count, 1, "a combinada needs two legs")
        XCTAssertEqual(d.tips.map(\.verdict), ["a favor", "en contra"])
        XCTAssertEqual(d.tips[0].evidence, "captura")
    }

    func testMikasModelSitsNextToParleysNumbers() throws {
        let d = try XCTUnwrap(ParleyDigest.parse(Self.sample, day: "d", generated: "g"))
        let a = d.matches[0]
        let x = try XCTUnwrap(a.model1X2)
        XCTAssertEqual(x.reduce(0, +), 1, accuracy: 1e-9)
        XCTAssertEqual(a.modelTotals?.count, 7)
        XCTAssertNotNil(a.markets[0].model, "Gana Austria is a 1X2 market")
        XCTAssertNotNil(a.markets[1].model, "Más de 2.5 goles is a totals market")
        XCTAssertNil(a.markets[3].model, "corners are not in the goals model")
        XCTAssertNotNil(a.pick?.model)
        XCTAssertNotNil(d.picks[0].model, "the day's pick finds its match's model")
        let p = try XCTUnwrap(d.matches[1].modelTennis)
        XCTAssertGreaterThan(p, 0.75)
        XCTAssertEqual(d.picks[1].model ?? 0, p, accuracy: 1e-9)
    }

    func testAnAnswerCutShortIsClosedAtItsLastCompleteValue() throws {
        let cut = """
        {"resumen":"x","partidos":[{"deporte":"futbol","local":"A","visita":"B","hora":"10:00"},{"deporte":"futbol","local":"C","visi
        """
        let d = try XCTUnwrap(ParleyDigest.parse(cut, day: "d", generated: "g"))
        XCTAssertEqual(d.matches.map(\.match), ["A vs B"])
        XCTAssertNil(ParleyDigest.repairTruncated("{\"a\":\"sin cerrar"))
    }

    func testTrendsDatesAndLevelAreRead() throws {
        let answer = """
        {"partidos":[{"deporte":"futbol","torneo":"Liga de Naciones","nivel":"fuerte","local":"Bélgica","visita":"Turquía","fecha":"2026-10-02","hora":"13:45",
          "tendencias":[{"texto":"Bélgica marcó 2+ en 8 de 10","aciertos":8,"de":10},{"texto":"sin tasa","aciertos":12,"de":10}],
          "pick":{"mercado":"Gana Bélgica","prob":0.6,"confianza":"alta"}}]}
        """
        let d = try XCTUnwrap(ParleyDigest.parse(answer, day: "2026-10-02", generated: "g"))
        let m = d.matches[0]
        XCTAssertEqual(m.level, "fuerte"); XCTAssertEqual(m.date, "2026-10-02")
        XCTAssertEqual(m.trends.count, 2)
        XCTAssertEqual(m.trends[0].hits, 8); XCTAssertEqual(m.trends[0].of, 10)
        XCTAssertNil(m.trends[1].hits, "12 of 10 is not a rate")
        XCTAssertEqual(ParleyDigest.when(date: "2026-10-02", time: "13:45"), "vie 2 oct · 13:45")
        XCTAssertTrue(ParleyDigest.isStrong("UEFA Nations League")); XCTAssertTrue(ParleyDigest.isStrong("Liga 1, Peru"))
        XCTAssertFalse(ParleyDigest.isStrong("Liga 3, Romania")); XCTAssertFalse(ParleyDigest.isStrong("Premier League 2"))
        XCTAssertFalse(ParleyDigest.isStrong("ATP Challenger Lima"))
        XCTAssertFalse(ParleyDigest.isStrong("LaLiga 2")); XCTAssertFalse(ParleyDigest.isStrong("Serie B de Brasil"))
        XCTAssertTrue(ParleyDigest.isStrong("LaLiga")); XCTAssertTrue(ParleyDigest.isStrong("Serie A"))
    }

    func testTheReviewsStartFromTheMorningAnalysis() throws {
        var d = try XCTUnwrap(ParleyDigest.parse(Self.sample, day: "2026-10-02", generated: "06:40"))
        d.matches[0].trends = [DigestTrend(text: "Austria marcó en 9 de 10", hits: 9, of: 10)]
        let window = ParleyDigest.base(d, from: "10:30", to: "12:30")
        XCTAssertTrue(window.contains("Austria vs Dinamarca")); XCTAssertFalse(window.contains("Sinner"), "05:00 is outside the window")
        XCTAssertTrue(window.contains("pick: Más de 2.5 goles")); XCTAssertTrue(window.contains("tendencias: Austria marcó en 9 de 10"))
        XCTAssertTrue(window.contains("06:40"))
        let picks = ParleyDigest.base(d)
        XCTAssertTrue(picks.contains("Austria vs Dinamarca")); XCTAssertTrue(picks.contains("Sinner vs Rune"))
        let review = Picks.reviewPrompt(rules: BettingRules(), now: "n", windowEnd: "12:30", used: 0, posts: [], timeZone: .current,
                                        media: URL(fileURLWithPath: "/m"), base: window)
        XCTAssertTrue(review.contains("Análisis de esta mañana")); XCTAssertTrue(review.contains("no repitas la investigación"))
        XCTAssertFalse(Picks.reviewPrompt(rules: BettingRules(), now: "n", windowEnd: "e", used: 0, posts: [], timeZone: .current,
                                          media: URL(fileURLWithPath: "/m")).contains("Análisis de esta mañana"))
    }

    func testLinksInsideTheTextKeepOnlyTheirWords() {
        XCTAssertEqual(ParleyDigest.plain("Suecia anotó 5. [Resultados UEFA](https://www.uefa.com/x) y más"), "Suecia anotó 5. Resultados UEFA y más")
        XCTAssertEqual(ParleyDigest.plain("[Previa](https://www.thestatszone.com/saint-lucia-vs"), "Previa", "cut short")
        XCTAssertEqual(ParleyDigest.plain("dato (https://x.com/a) fin"), "dato fin")
    }

    func testNothingUsableIsNil() {
        XCTAssertNil(ParleyDigest.parse("No pude buscar hoy.", day: "d", generated: "g"))
        XCTAssertNil(ParleyDigest.parse("{\"resumen\":\"x\"}", day: "d", generated: "g"))
    }

    func testThePageEscapesEverythingAndOnlyLinksHttp() throws {
        var d = try XCTUnwrap(ParleyDigest.parse(Self.sample, day: "2026-10-02", generated: "07:40"))
        d.matches[0].sources.append("javascript:alert(1)")
        let page = ParleyDigest.html(d)
        XCTAssertFalse(page.contains("<b>Rune</b>"), "model text is never HTML")
        XCTAssertTrue(page.contains("&lt;b&gt;Rune&lt;/b&gt;"))
        XCTAssertFalse(page.contains("javascript:"))
        XCTAssertFalse(page.contains("<script"))
        XCTAssertTrue(page.contains("Content-Security-Policy"))
        XCTAssertTrue(page.contains("rel=\"noopener noreferrer\""))
        XCTAssertTrue(page.contains("<svg"), "the page has charts")
        XCTAssertTrue(page.contains("id=\"theme\""), "light/dark switch")
        XCTAssertTrue(page.contains("Bélgica más de 2.5"), "Telegram picks are on the page")
        XCTAssertTrue(page.contains("id=\"f-fuerte\""), "league filters")
        XCTAssertTrue(page.contains("id=\"o-valor\""), "order switch")
        XCTAssertTrue(page.contains("data-f=\"futbol"), "cards carry their filter tokens")
        XCTAssertTrue(page.contains("Frente a otros mercados"))
        XCTAssertTrue(page.contains("Por qué se cumple") || d.matches.allSatisfy { $0.trends.isEmpty })
    }

    func testFallbackKeepsTheTextEscaped() {
        let page = ParleyDigest.fallbackHTML(day: "2026-10-02", generated: "11:20", text: "hola <script>x</script>\nsegunda")
        XCTAssertFalse(page.contains("<script>x"))
        XCTAssertTrue(page.contains("hola &lt;script&gt;x&lt;/script&gt;"))
    }

    func testTheChatSummaryListsTheBestPicksAndThePriceToLookFor() throws {
        let d = try XCTUnwrap(ParleyDigest.parse(Self.sample, day: "d", generated: "g"))
        let text = ParleyDigest.chatSummary(d)
        XCTAssertTrue(text.contains("2 partidos"))
        XCTAssertTrue(text.contains("1. Austria vs Dinamarca: Más de 2.5 goles (juega desde @1.82) · 55 %"))
        XCTAssertTrue(text.contains("2. Sinner vs Rune: Gana Sinner @1.25 · 78 %"))
        XCTAssertTrue(text.contains("Telegram: 1 a favor"))
    }

    func testDueOnlyFromTheHourAndOncePerDay() {
        let tz = TimeZone(identifier: "America/Lima")!
        func at(_ h: Int, _ m: Int) -> Date {
            var c = Calendar(identifier: .gregorian); c.timeZone = tz
            return c.date(from: DateComponents(year: 2026, month: 10, day: 2, hour: h, minute: m))!
        }
        XCTAssertNil(ParleyDigest.due(now: at(6, 59), timeZone: tz, hour: 7, marker: nil), "the hour the user set")
        XCTAssertEqual(ParleyDigest.due(now: at(7, 0), timeZone: tz, hour: 7, marker: nil), "2026-10-02")
        XCTAssertEqual(ParleyDigest.due(now: at(23, 30), timeZone: tz, hour: 7, marker: "{\"day\":\"2026-10-01\"}"), "2026-10-02")
        XCTAssertNil(ParleyDigest.due(now: at(15, 0), timeZone: tz, hour: 7, marker: "{\"day\":\"2026-10-02\"}"))
        XCTAssertEqual(ParleyDigest.startOfDay(at(15, 0), timeZone: tz), Int64(at(0, 0).timeIntervalSince1970))
    }

    func testTheDayIsOneTurnBuiltOnTheTelegramPicks() {
        let rules = BettingRules(bankroll: 400, minOdds: 1.5)
        let posts = ParleyDigest.postsText([TelegramPost(chatId: 1, chat: "JC", id: 2, date: 1_790_950_000, text: "Bélgica +2.5", sender: "Juan",
                                                        photos: ["1_2.jpg"])], timeZone: .current, media: URL(fileURLWithPath: "/m"))
        XCTAssertTrue(posts.contains("canal: JC · autor: Juan")); XCTAssertTrue(posts.contains("captura: /m/1_2.jpg"))
        let p = ParleyDigest.dayPrompt(rules: rules, day: "2026-10-02", now: "n", from: "06:00", posts: posts,
                                       prices: ["Mainz vs Werder (Bundesliga) · 13:30 · Full Time Result: 1 @2.15"], hasForm: true)
        XCTAssertTrue(p.contains("UNA sola pasada"))
        XCTAssertTrue(p.contains("La base son los picks de los canales de Telegram"))
        XCTAssertTrue(p.contains("canal: JC"))
        XCTAssertTrue(p.contains("hasta 12 partidos")); XCTAssertTrue(p.contains("entre las 06:00 y las 22:00"))
        XCTAssertTrue(p.contains("como mucho 20 en total"))
        XCTAssertTrue(p.contains("Mainz vs Werder (Bundesliga) · 13:30 · Full Time Result: 1 @2.15"))
        XCTAssertTrue(p.contains("Siempre das pick")); XCTAssertTrue(p.contains("No comentes la hora de las cuotas"))
        XCTAssertTrue(p.contains("\"tendencias\"")); XCTAssertTrue(p.contains("```json"))
        XCTAssertTrue(ParleyDigest.dayPrompt(rules: rules, day: "d", now: "n", from: "10:00", posts: "", prices: [], hasForm: false)
            .contains("no publicaron nada"))
        let list = ParleyDigest.oddsList("""
        # Partidos de hoy
        - SK Sigma vs MSK Zilina (Club Friendly Games) · 07:00
          Full Time Result: 1 @1.67 · X @3.80 · 2 @4.00
        - Mainz vs Werder (Bundesliga) · 13:30
          Full Time Result: 1 @2.15 · X @3.50 · 2 @3.30
          Both Teams To Score: Yes @1.72
        - Roma vs Lazio (Serie A) · 23:00
          Full Time Result: 1 @2.0
        """, from: "10:00")
        XCTAssertEqual(list, ["Mainz vs Werder (Bundesliga) · 13:30 · Full Time Result: 1 @2.15 · X @3.50 · 2 @3.30"],
                       "only what is left of the day, one line per match")
    }

    func testThePicksYourChannelsBackComeFirst() {
        func match(_ home: String, _ away: String, conf: String) -> DigestMatch {
            var m = DigestMatch(sport: "futbol", tournament: "L", match: "\(home) vs \(away)", home: home, away: away, time: "20:00", odds: "")
            m.pick = DigestMatchPick(market: "Gana \(home)", prob: 0.6, odds: 1.9, confidence: conf, reason: "r")
            return m
        }
        let tip = DigestTip(channel: "JC", author: "", time: "", pick: "Sporting Cristal gana", odds: 1.9, evidence: "texto", prob: 0.6,
                            verdict: "a favor", reason: "")
        let d = ParleyDigest.assemble(matches: [match("Mainz", "Werder", conf: "alta"), match("Sporting Cristal", "Cusco", conf: "media")],
                                      tips: [tip], rules: BettingRules(minOdds: 1.5), used: 0, day: "d", generated: "g", notes: [])
        XCTAssertEqual(d.picks.map(\.match), ["Sporting Cristal vs Cusco", "Mainz vs Werder"], "the paid channels' pick leads")
    }

    func testMikaRanksThePicksSetsTheStakesAndBuildsTheCombinadas() throws {
        let sample = try XCTUnwrap(ParleyDigest.parse(Self.sample, day: "d", generated: "g"))
        func match(_ home: String, _ away: String, market: String, prob: Double, odds: Double?, conf: String) -> DigestMatch {
            var m = DigestMatch(sport: "futbol", tournament: "L", match: "\(home) vs \(away)", home: home, away: away, time: "20:00", odds: "")
            m.pick = DigestMatchPick(market: market, prob: prob, odds: odds, confidence: conf, reason: "r")
            return m
        }
        let matches = [sample.matches[0],
                       match("C", "D", market: "Gana C", prob: 0.62, odds: 1.80, conf: "alta"),
                       match("E", "F", market: "Más de 1.5 goles", prob: 0.80, odds: 1.20, conf: "alta"),   // under the minimum odds
                       match("G", "H", market: "Gana G", prob: 0.55, odds: nil, conf: "media"),
                       match("I", "J", market: "Empate", prob: 0.30, odds: 3.60, conf: "baja")]
        let rules = BettingRules(bankroll: 400, stakeMinPct: 1, stakeMaxPct: 3, dailyCapPct: 10, minOdds: 1.5)
        let d = ParleyDigest.assemble(matches: matches, tips: sample.tips, rules: rules, used: 0, day: "2026-10-02", generated: "08:10",
                                      notes: ["El lote 3 no terminó: x"])
        XCTAssertEqual(d.picks.map(\.match), ["C vs D", "Austria vs Dinamarca", "G vs H", "I vs J"], "alta first, then media, then baja; 1.20 is out")
        XCTAssertEqual(d.picks[0].stake, "3 % de la banca", "alta: the top of the range")
        XCTAssertEqual(d.picks[1].stake, "2 % de la banca", "media: the middle")
        XCTAssertEqual(d.picks[3].stake, "1 % de la banca", "baja: the bottom")
        XCTAssertNotNil(d.picks[1].model, "MIKA's model travels with the pick")
        XCTAssertFalse(d.combos.isEmpty)
        XCTAssertTrue(d.combos.allSatisfy { $0.legs.count >= 2 })
        XCTAssertTrue(d.summary.contains("Analicé 5 partidos"))
        XCTAssertEqual(d.warnings, ["El lote 3 no terminó: x"])
        XCTAssertNotNil(d.matches[0].model1X2, "every match gets MIKA's model")
        let capped = ParleyDigest.assemble(matches: matches, tips: [], rules: BettingRules(bankroll: 0, stakeMinPct: 1, stakeMaxPct: 10,
                                                                                         dailyCapPct: 12, minOdds: 1.5),
                                           used: 0, day: "d", generated: "g", notes: [])
        XCTAssertEqual(capped.picks.map(\.stake), ["10 % de la banca", "2 % de la banca", "0 % de la banca", "0 % de la banca"],
                       "never past the daily cap")
    }

    func testRulesKeepTheDigestSettingsAndOldFilesGetTheDefaults() throws {
        var rules = BettingRules(); rules.digestHour = 99
        XCTAssertEqual(rules.sanitized().digestHour, 23)
        let old = try JSONDecoder().decode(BettingRules.self, from: Data("{\"autoScan\":true}".utf8))
        XCTAssertTrue(old.dailyDigest); XCTAssertEqual(old.digestHour, 6)
    }

    func testTheShippedParleyIsUpgradedToTheAnalyst() {
        XCTAssertEqual(AgentStore.upgrade(id: "parley", installed: AgentStore.parleyBeforeAnalyst, to: AgentStore.parleyDefault),
                       AgentStore.parleyDefault)
        XCTAssertTrue(AgentStore.parleyDefault.contains("Siempre entregas picks"))
    }

    func testAChatPageSurvivesSavingAndOldHistoriesStillLoad() throws {
        let m = ChatMessage(role: .assistant, content: "x", page: ChatPage(title: "Abrir resumen", path: "/a/digest/d.html"))
        let back = try JSONDecoder().decode(ChatMessage.self, from: JSONEncoder().encode(m))
        XCTAssertEqual(back.page, m.page)
        let old = try JSONDecoder().decode(ChatMessage.self, from: Data("{\"role\":\"assistant\",\"content\":\"y\"}".utf8))
        XCTAssertNil(old.page)
    }
}
