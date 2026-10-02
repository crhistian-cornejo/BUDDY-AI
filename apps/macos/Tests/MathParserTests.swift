// Ported from MIKA (MIT, revision d050bc5).
import XCTest

final class MathParserTests: XCTestCase {
    private func sym(_ s: String, italic: Bool = false) -> MathNode { .symbol(s, italic: italic) }
    private func letter(_ s: String) -> MathNode { .symbol(s, italic: true) }

    func testLettersDigitsAndOperators() {
        XCTAssertEqual(MathParser.parse("12.5"), sym("12.5"))
        XCTAssertEqual(MathParser.parse("a-b"), .row([letter("a"), sym("−"), letter("b")]))
        XCTAssertEqual(MathParser.parse("\\alpha + \\beta"), .row([letter("α"), sym("+"), letter("β")]))
        XCTAssertEqual(MathParser.parse("\\Gamma"), sym("Γ"), "las mayúsculas griegas van rectas")
    }

    func testScripts() {
        XCTAssertEqual(MathParser.parse("x^2"), .script(base: letter("x"), sup: sym("2"), sub: nil))
        XCTAssertEqual(MathParser.parse("a_i^{n+1}"),
                       .script(base: letter("a"), sup: .row([letter("n"), sym("+"), sym("1")]), sub: letter("i")))
        XCTAssertEqual(MathParser.parse("x^23"), .row([.script(base: letter("x"), sup: sym("2"), sub: nil), sym("3")]),
                       "un exponente sin llaves es un solo carácter")
    }

    func testFractionsRootsAndText() {
        XCTAssertEqual(MathParser.parse("\\frac{a}{b}"), .fraction(letter("a"), letter("b")))
        XCTAssertEqual(MathParser.parse("\\sqrt{x}"), .sqrt(letter("x"), index: nil))
        XCTAssertEqual(MathParser.parse("\\sqrt[3]{x}"), .sqrt(letter("x"), index: sym("3")))
        XCTAssertEqual(MathParser.parse("\\text{si } x"), .row([.text("si "), letter("x")]))
    }

    func testBigOperatorsKeepTheirLimits() {
        XCTAssertEqual(MathParser.parse("\\sum_{i=1}^{n} i"),
                       .row([.bigOp("∑", sub: .row([letter("i"), sym("="), sym("1")]), sup: letter("n")), letter("i")]))
        XCTAssertNotNil(MathParser.parse("\\int_0^1 x\\,dx"))
        XCTAssertNotNil(MathParser.parse("\\lim_{x \\to \\infty} f(x)"))
    }

    func testDelimitersAndMatrices() {
        XCTAssertEqual(MathParser.parse("\\left( x \\right)"), .delimited(left: "(", right: ")", letter("x")))
        XCTAssertEqual(MathParser.parse("\\begin{pmatrix} a & b \\\\ c & d \\end{pmatrix}"),
                       .matrix(rows: [[letter("a"), letter("b")], [letter("c"), letter("d")]], left: "(", right: ")", alignment: .center))
        XCTAssertEqual(MathParser.parse("\\begin{cases} 1 & x > 0 \\\\ 0 & x \\le 0 \\end{cases}").map { node -> String in
            if case .matrix(_, let l, let r, _) = node { return l + "|" + r } else { return "?" } }, "{|")
    }

    func testStylesAndSymbols() {
        XCTAssertEqual(MathParser.parse("\\mathbb{R}"), sym("ℝ"))
        XCTAssertEqual(MathParser.parse("\\vec{v}"), .accent(.vec, letter("v")))
        XCTAssertEqual(MathParser.parse("a \\leq b"), .row([letter("a"), sym("≤"), letter("b")]))
        XCTAssertEqual(MathParser.parse("\\sin x"), .row([sym("sin"), letter("x")]))
    }

    func testAnythingItDoesNotUnderstandIsNilSoTheSourceCanBeShown() {
        XCTAssertNil(MathParser.parse("\\foo{x}"))
        XCTAssertNil(MathParser.parse("\\frac{a}"))
        XCTAssertNil(MathParser.parse("x^"))
        XCTAssertNil(MathParser.parse("{x"))
        XCTAssertNil(MathParser.parse("x}"))
        XCTAssertNil(MathParser.parse("\\begin{weird} a \\end{weird}"))
    }

    func testPlainTextForFallbacksAndInlineUse() {
        XCTAssertEqual(MathParser.parse("\\frac{a}{b} + x^2").map(\.plainText), "a⁄b + x²")
        XCTAssertEqual(MathParser.parse("\\sqrt{x}").map(\.plainText), "√x")
        XCTAssertEqual(MathParser.parse("\\sum_{i=1}^{n} i").map(\.plainText), "∑ᵢ₌₁ⁿ i")
    }
}
