// Ported from MIKA (MIT, revision d050bc5).
import XCTest
@testable import Buddy

final class MarkdownParserTests: XCTestCase {
    private func item(_ text: String, _ children: [MarkdownBlock] = []) -> MarkdownListItem { MarkdownListItem(text: text, children: children) }

    func testHeadingsAndParagraphs() {
        XCTAssertEqual(MarkdownParser.parse("# Título\n\nTexto **negrita**"),
                       [.heading(level: 1, text: "Título"), .paragraph("Texto **negrita**")])
        XCTAssertEqual(MarkdownParser.parse("### Tres #"), [.heading(level: 3, text: "Tres")])
        XCTAssertEqual(MarkdownParser.parse("#sinespacio"), [.paragraph("#sinespacio")], "sin espacio no es un título")
    }

    func testSingleNewlinesInAParagraphAreKept() {
        XCTAssertEqual(MarkdownParser.parse("línea uno\nlínea dos"), [.paragraph("línea uno\nlínea dos")])
        XCTAssertEqual(MarkdownParser.parse("a\r\nb"), [.paragraph("a\nb")])
    }

    func testNestedBulletList() {
        let md = "- uno\n- dos\n  - dos.a\n  - dos.b\n- tres"
        XCTAssertEqual(MarkdownParser.parse(md), [
            .bulletList([item("uno"),
                         item("dos", [.bulletList([item("dos.a"), item("dos.b")])]),
                         item("tres")])
        ])
    }

    func testOrderedListKeepsItsFirstNumberAndAListCanInterruptAParagraph() {
        XCTAssertEqual(MarkdownParser.parse("3. a\n4. b"), [.orderedList(start: 3, items: [item("a"), item("b")])])
        XCTAssertEqual(MarkdownParser.parse("Pasos:\n- a\n- b"), [.paragraph("Pasos:"), .bulletList([item("a"), item("b")])])
    }

    func testCodeBlocksAreVerbatimAndAnUnclosedOneStillRendersWhileStreaming() {
        XCTAssertEqual(MarkdownParser.parse("```swift\nlet x = 1\n- no es lista\n$$ no es mates\n```\nfin"),
                       [.codeBlock(language: "swift", code: "let x = 1\n- no es lista\n$$ no es mates"), .paragraph("fin")])
        XCTAssertEqual(MarkdownParser.parse("```\nsin cerrar"), [.codeBlock(language: nil, code: "sin cerrar")])
    }

    func testDisplayMathInItsThreeSpellings() {
        XCTAssertEqual(MarkdownParser.parse("$$\\frac{a}{b}$$"), [.math("\\frac{a}{b}")])
        XCTAssertEqual(MarkdownParser.parse("$$\nx = 1\n$$"), [.math("x = 1")])
        XCTAssertEqual(MarkdownParser.parse("\\[ E = mc^2 \\]"), [.math("E = mc^2")])
        XCTAssertEqual(MarkdownParser.parse("antes\n$$\na\nb\n$$\ndespués"), [.paragraph("antes"), .math("a\nb"), .paragraph("después")])
    }

    func testAnUnclosedDisplayMathStaysTextUntilTheClosingDelimiterArrives() {
        XCTAssertEqual(MarkdownParser.parse("$$\\frac{a}"), [.paragraph("$$\\frac{a}")])
    }

    func testQuoteRuleAndTable() {
        XCTAssertEqual(MarkdownParser.parse("> cita\n> más"), [.quote([.paragraph("cita\nmás")])])
        XCTAssertEqual(MarkdownParser.parse("---"), [.rule])
        XCTAssertEqual(MarkdownParser.parse("| a | b |\n|:--|--:|\n| 1 | 2 |\n| 3 | 4 |"),
                       [.table(header: ["a", "b"], alignments: [.leading, .trailing], rows: [["1", "2"], ["3", "4"]])])
        XCTAssertEqual(MarkdownParser.parse("| solo cabecera |"), [.paragraph("| solo cabecera |")], "sin separador no es tabla")
    }

    func testEmptyAndWhitespaceOnlyInputGiveNoBlocks() {
        XCTAssertEqual(MarkdownParser.parse(""), [])
        XCTAssertEqual(MarkdownParser.parse("  \n \n"), [])
    }
}

final class MarkdownInlineTests: XCTestCase {
    func testMoneyIsNotMathButARealFormulaIs() {
        XCTAssertEqual(MarkdownInline.segments("precio $5 y $10"), [.text("precio $5 y $10")])
        XCTAssertEqual(MarkdownInline.segments("si $x^2$ entonces"), [.text("si "), .math("x^2"), .text(" entonces")])
    }

    func testParenthesisAndDoubleDollarSpellings() {
        XCTAssertEqual(MarkdownInline.segments("\\(a+b\\)"), [.math("a+b")])
        XCTAssertEqual(MarkdownInline.segments("ver $$x$$ ahora"), [.text("ver "), .math("x"), .text(" ahora")])
    }

    func testCodeSpansAndEscapedDollarsAreNeverMath() {
        XCTAssertEqual(MarkdownInline.segments("`$x$` y $y$"), [.text("`$x$` y "), .math("y")])
        XCTAssertEqual(MarkdownInline.segments("cuesta \\$5 y \\$6"), [.text("cuesta \\$5 y \\$6")])
    }

    func testUnclosedOrMultilineDollarStaysText() {
        XCTAssertEqual(MarkdownInline.segments("$x sin cerrar"), [.text("$x sin cerrar")])
        XCTAssertEqual(MarkdownInline.segments("$a\nb$"), [.text("$a\nb$")])
    }

    func testTaskItemsAreRecognisedLikeOnWindows() {
        XCTAssertEqual(MarkdownTask.marker("[ ] pendiente")?.checked, false)
        XCTAssertEqual(MarkdownTask.marker("[ ] pendiente")?.rest, "pendiente")
        XCTAssertEqual(MarkdownTask.marker("[x] hecho\nmás")?.checked, true)
        XCTAssertEqual(MarkdownTask.marker("[X] hecho")?.rest, "hecho")
        XCTAssertNil(MarkdownTask.marker("[x]sin espacio"))
        XCTAssertNil(MarkdownTask.marker("texto [ ] medio"))
        XCTAssertNil(MarkdownTask.marker("[ ]"))
    }
}
