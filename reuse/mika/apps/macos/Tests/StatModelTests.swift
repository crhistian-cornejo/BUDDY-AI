import XCTest

final class StatModelTests: XCTestCase {
    func testPoissonGridAddsUpAndFavoursTheStrongerSide() throws {
        let m = try XCTUnwrap(StatModel.football(homeFor: 2.2, homeAgainst: 0.8, awayFor: 0.9, awayAgainst: 1.8))
        XCTAssertEqual(m.home, 2.0, accuracy: 1e-9)
        XCTAssertEqual(m.away, 0.85, accuracy: 1e-9)
        XCTAssertEqual(m.homeWin + m.draw + m.awayWin, 1, accuracy: 1e-9)
        XCTAssertEqual(m.totals.reduce(0, +), 1, accuracy: 1e-9)
        XCTAssertGreaterThan(m.homeWin, 0.55)
        XCTAssertLessThan(m.awayWin, 0.2)
        XCTAssertEqual(m.over(2.5), 1 - m.totals[0] - m.totals[1] - m.totals[2], accuracy: 1e-9)
        XCTAssertEqual(m.likelyScores.count, 3)
    }

    func testEvenSidesGiveTheTextbookNumbers() throws {
        // λ = 1.35 each side: P(draw) ≈ 0.258 and P(over 2.5) ≈ 0.506 (independent Poisson).
        let m = try XCTUnwrap(StatModel.football(homeFor: 1.35, homeAgainst: 1.35, awayFor: 1.35, awayAgainst: 1.35))
        XCTAssertEqual(m.homeWin, m.awayWin, accuracy: 1e-9)
        XCTAssertEqual(m.draw, 0.258, accuracy: 0.005)
        XCTAssertEqual(m.over(2.5), 0.506, accuracy: 0.005)
    }

    func testMissingOrAbsurdRatesGiveNoModel() {
        XCTAssertNil(StatModel.football(homeFor: nil, homeAgainst: 1, awayFor: 1, awayAgainst: 1))
        XCTAssertNil(StatModel.football(homeFor: 40, homeAgainst: 1, awayFor: 1, awayAgainst: 1))
    }

    func testTennisFromRankingAndSurface() throws {
        let even = try XCTUnwrap(StatModel.tennis(rankA: 20, rankB: 20, surfaceA: nil, surfaceB: nil))
        XCTAssertEqual(even, 0.5, accuracy: 1e-9)
        let better = try XCTUnwrap(StatModel.tennis(rankA: 10, rankB: 20, surfaceA: nil, surfaceB: nil))
        XCTAssertEqual(better, 0.643, accuracy: 0.005)
        let surface = try XCTUnwrap(StatModel.tennis(rankA: nil, rankB: nil, surfaceA: 70, surfaceB: 0.5))
        XCTAssertEqual(surface, 0.7, accuracy: 1e-9)
        XCTAssertNil(StatModel.tennis(rankA: nil, rankB: 3, surfaceA: nil, surfaceB: nil))
    }

    func testPriceArithmetic() {
        XCTAssertEqual(StatModel.implied(2.0) ?? 0, 0.5, accuracy: 1e-9)
        XCTAssertNil(StatModel.implied(nil))
        XCTAssertEqual(StatModel.fairOdds(0.25) ?? 0, 4, accuracy: 1e-9)
        XCTAssertEqual(StatModel.edge(probability: 0.55, odds: 2.0) ?? 0, 0.10, accuracy: 1e-9)
        XCTAssertNil(StatModel.edge(probability: 0.55, odds: nil))
    }

    func testMarketNamesMapToTheGrid() throws {
        let m = try XCTUnwrap(StatModel.football(homeFor: 1.6, homeAgainst: 1.1, awayFor: 1.2, awayAgainst: 1.4))
        func p(_ s: String) -> Double? { StatModel.probability(of: s, home: "Austria", away: "Dinamarca", model: m) }
        XCTAssertEqual(p("Más de 2.5 goles") ?? -1, m.over(2.5), accuracy: 1e-9)
        XCTAssertEqual(p("Menos de 3,5 goles") ?? -1, 1 - m.over(3.5), accuracy: 1e-9)
        XCTAssertEqual(p("Ambos marcan: Sí") ?? -1, m.bothScore, accuracy: 1e-9)
        XCTAssertEqual(p("Gana Austria") ?? -1, m.homeWin, accuracy: 1e-9)
        XCTAssertEqual(p("Gana Dinamarca") ?? -1, m.awayWin, accuracy: 1e-9)
        XCTAssertEqual(p("Empate") ?? -1, m.draw, accuracy: 1e-9)
        XCTAssertEqual(p("Doble oportunidad 1X") ?? -1, m.homeWin + m.draw, accuracy: 1e-9)
        XCTAssertNil(p("Hándicap asiático Austria -1"))
        XCTAssertNil(p("Empate al descanso"), "half-time markets are not the full-match grid")
        XCTAssertNil(p("Más de 0.5 goles primer tiempo"))
        let short = StatModel.probability(of: "Correcaminos gana", home: "Correcaminos UAT", away: "Cruz Azul Hidalgo", model: m)
        XCTAssertEqual(short ?? -1, m.homeWin, accuracy: 1e-9, "a word of the name is enough")
        XCTAssertEqual(StatModel.probability(of: "Ferro gana", home: "Ferro Carril Oeste", away: "Deportivo Madryn", model: m) ?? -1, m.homeWin, accuracy: 1e-9)
        XCTAssertNil(StatModel.probability(of: "Real gana", home: "Real Madrid", away: "Real Sociedad", model: m), "a shared word names nobody")
        XCTAssertNil(p("Más de 9.5 córners"))
    }
}
