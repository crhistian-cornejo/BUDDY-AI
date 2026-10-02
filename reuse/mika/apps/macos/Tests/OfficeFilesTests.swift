import XCTest

final class OfficeFilesTests: XCTestCase {
    private func json(_ text: String) -> JSONValue { try! JSONDecoder().decode(JSONValue.self, from: Data(text.utf8)) }

    private var workspace: URL!

    override func setUp() {
        workspace = FileManager.default.temporaryDirectory.appendingPathComponent("office-\(UUID().uuidString)", isDirectory: true)
        try? FileManager.default.createDirectory(at: workspace, withIntermediateDirectories: true)
    }

    override func tearDown() { try? FileManager.default.removeItem(at: workspace) }

    /// The names and the bytes of a ZIP, read back with an independent little parser of the central directory.
    private func entries(_ zip: Data) -> [(name: String, crc: UInt32, size: Int)] {
        func u16(_ o: Int) -> Int { Int(zip[o]) | Int(zip[o + 1]) << 8 }
        func u32(_ o: Int) -> UInt32 { UInt32(zip[o]) | UInt32(zip[o + 1]) << 8 | UInt32(zip[o + 2]) << 16 | UInt32(zip[o + 3]) << 24 }
        var end = zip.count - 22
        while end > 0, u32(end) != 0x0605_4B50 { end -= 1 }
        let count = u16(end + 10)
        var offset = Int(u32(end + 16))
        var out: [(String, UInt32, Int)] = []
        for _ in 0..<count {
            XCTAssertEqual(u32(offset), 0x0201_4B50)
            let nameLength = u16(offset + 28), extra = u16(offset + 30), comment = u16(offset + 32)
            out.append((String(decoding: zip[(offset + 46)..<(offset + 46 + nameLength)], as: UTF8.self), u32(offset + 16), Int(u32(offset + 24))))
            offset += 46 + nameLength + extra + comment
        }
        return out
    }

    func testTheZipIsWellFormed() {
        XCTAssertEqual(Zip.crc32(Data("123456789".utf8)), 0xCBF4_3926, "the standard CRC-32 check value")
        let zip = Zip.archive([("a.txt", Data("hola".utf8)), ("dir/b.xml", Data("<x/>".utf8))])
        let found = entries(zip)
        XCTAssertEqual(found.map(\.name), ["a.txt", "dir/b.xml"])
        XCTAssertEqual(found[0].crc, Zip.crc32(Data("hola".utf8)))
        XCTAssertEqual(found[1].size, 4)
        XCTAssertEqual(Zip.archive([("a.txt", Data("hola".utf8))]), Zip.archive([("a.txt", Data("hola".utf8))]), "the same input is the same file")
    }

    func testXMLTextIsEscapedAndControlCharactersDropped() {
        XCTAssertEqual(OfficeXML.escape("a & b < c > \"d\""), "a &amp; b &lt; c &gt; &quot;d&quot;")
        XCTAssertEqual(OfficeXML.escape("a\u{0}b\u{8}c\td\n"), "abc\td\n")
        XCTAssertEqual(OfficeXML.column(0), "A"); XCTAssertEqual(OfficeXML.column(25), "Z"); XCTAssertEqual(OfficeXML.column(26), "AA"); XCTAssertEqual(OfficeXML.column(701), "ZZ")
        XCTAssertEqual(OfficeXML.runs("a **b** *c* `d` e"), [
            .init(text: "a "), .init(text: "b", bold: true), .init(text: " "), .init(text: "c", italic: true), .init(text: " "), .init(text: "d", code: true), .init(text: " e")])
        XCTAssertEqual(OfficeXML.runs("2 * 3 = 6").map(\.text).joined(), "2 * 3 = 6", "a lone asterisk is just text")
    }

    func testFileNamesStayInTheFolderAndNeverOverwrite() throws {
        XCTAssertEqual(try OfficeTools.fileName("Informe 2026", ext: "docx"), "Informe 2026.docx")
        XCTAssertEqual(try OfficeTools.fileName("informe.xlsx", ext: "docx"), "informe.docx", "the extension is the tool's")
        XCTAssertEqual(try OfficeTools.fileName("../../etc/passwd", ext: "docx"), "passwd.docx")
        XCTAssertEqual(try OfficeTools.fileName("a\\b\\c:d?.pptx", ext: "pptx"), "c_d.pptx", "unsafe characters become _, and the ends are trimmed")
        XCTAssertEqual(try OfficeTools.fileName(String(repeating: "x", count: 200), ext: "docx").count, 85)
        XCTAssertThrowsError(try OfficeTools.fileName("", ext: "docx"))
        XCTAssertThrowsError(try OfficeTools.fileName("...", ext: "docx"))
        let args = json(#"{"file":"x.docx","blocks":[{"type":"paragraph","text":"hola"}]}"#)
        let first = try OfficeTools.run(OfficeTools.document, args: args, workspace: workspace)
        let second = try OfficeTools.run(OfficeTools.document, args: args, workspace: workspace)
        XCTAssertEqual(first.lastPathComponent, "x.docx")
        XCTAssertEqual(second.lastPathComponent, "x (2).docx")
        XCTAssertEqual(first.deletingLastPathComponent().lastPathComponent, OfficeTools.folder)
    }

    func testTheDocumentHasItsPartsStylesAndContent() throws {
        let url = try OfficeTools.run(OfficeTools.document, args: json(#"""
        {"file":"d.docx","title":"Título","blocks":[{"type":"heading","level":1,"text":"Uno"},{"type":"paragraph","text":"a **b** & c"},
          {"type":"bullets","items":["x","y"]},{"type":"numbered","items":["1"]},{"type":"numbered","items":["2"]},
          {"type":"table","header":["A","B"],"rows":[["1","2"]]},{"type":"pagebreak"}]}
        """#), workspace: workspace)
        let zip = try Data(contentsOf: url)
        XCTAssertEqual(entries(zip).map(\.name), ["[Content_Types].xml", "_rels/.rels", "word/document.xml", "word/_rels/document.xml.rels", "word/styles.xml", "word/numbering.xml"])
        let text = String(decoding: zip, as: UTF8.self)
        for needle in ["w:pStyle w:val=\"Title\"", "w:pStyle w:val=\"Heading1\"", " &amp; c", "<w:tbl>", "w:br w:type=\"page\"", "<w:numId w:val=\"1\"/>", "<w:numId w:val=\"2\"/>", "<w:numId w:val=\"3\"/>"] {
            XCTAssertTrue(text.contains(needle), needle)
        }
        XCTAssertThrowsError(try OfficeTools.parseBlocks(json(#"[{"type":"video"}]"#)))
        XCTAssertThrowsError(try OfficeTools.parseBlocks(json("[]")))
    }

    func testTheWorkbookKeepsFormulasTypesAndASafeSheetName() throws {
        let sheets = try OfficeTools.parseSheets(json(#"""
        [{"name":"Ventas: Q1/2026 con un nombre larguísimo","rows":[["N","V"],["a",1.5],["b","=SUM(B2:B2)"],[true,null]]},{"rows":[["x"]],"header":false}]
        """#))
        XCTAssertEqual(sheets[0].name.count, 31)
        XCTAssertFalse(sheets[0].name.contains(":") || sheets[0].name.contains("/"))
        XCTAssertEqual(sheets[1].name, "Hoja2")
        XCTAssertEqual(sheets[0].rows[2][1], .formula("SUM(B2:B2)"))
        XCTAssertEqual(sheets[0].rows[3], [.bool(true), .empty])
        let text = XlsxWriter.parts(sheets: sheets).map { String(decoding: $0.data, as: UTF8.self) }.joined(separator: "\n")
        XCTAssertTrue(text.contains("<f>SUM(B2:B2)</f>") && text.contains("t=\"inlineStr\"") && text.contains("<v>1.5</v>") && text.contains("t=\"b\""))
        XCTAssertTrue(text.contains("<autoFilter ref=\"A1:B4\"/>") && text.contains("state=\"frozen\""))
        XCTAssertThrowsError(try OfficeTools.parseSheets(json(#"[{"name":"x"}]"#)))
    }

    func testThePresentationHasACoverAndContentSlides() throws {
        let slides = try OfficeTools.parseSlides(json(#"[{"title":"Plan","subtitle":"2026"},{"title":"Metas","bullets":["a","**b**"]}]"#))
        let parts = PptxWriter.parts(slides: slides)
        let names = parts.map(\.path)
        for name in ["ppt/presentation.xml", "ppt/slideMasters/slideMaster1.xml", "ppt/slideLayouts/slideLayout1.xml", "ppt/slideLayouts/slideLayout2.xml", "ppt/theme/theme1.xml", "ppt/slides/slide1.xml", "ppt/slides/slide2.xml", "ppt/slides/_rels/slide2.xml.rels"] {
            XCTAssertTrue(names.contains(name), name)
        }
        func text(_ path: String) -> String { String(decoding: parts.first { $0.path == path }!.data, as: UTF8.self) }
        XCTAssertTrue(text("ppt/slides/_rels/slide1.xml.rels").contains("slideLayout1.xml"), "the cover uses the cover layout")
        XCTAssertTrue(text("ppt/slides/_rels/slide2.xml.rels").contains("slideLayout2.xml"))
        XCTAssertTrue(text("ppt/slides/slide2.xml").contains("b=\"1\""))
        XCTAssertThrowsError(try OfficeTools.parseSlides(json("[]")))
    }

    func testOnlyAgentsWithTheCapabilityGetTheOfficeTools() throws {
        let maki = try AgentDefinition.parse(AgentStore.makiDefault), mira = try AgentDefinition.parse(AgentStore.miraDefault)
        XCTAssertTrue(maki.can.contains(.office) && mira.can.contains(.office), "MAKI and MIRA wrote documents already")
        for other in [AgentStore.mikaDefault, AgentStore.miroDefault, AgentStore.midaDefault, AgentStore.parleyDefault] {
            XCTAssertFalse(try AgentDefinition.parse(other).can.contains(.office), "the rest do not")
        }
        XCTAssertTrue(AgentTools.codexTools(for: maki).contains("create_spreadsheet"))
        XCTAssertFalse(AgentTools.codexTools(for: try AgentDefinition.parse(AgentStore.mikaDefault)).contains("create_document"))
        let fed = try AgentDefinition.parse("---\nid: other\nname: O\nintegration: telegram\ncan: [read, office]\n---\n")
        XCTAssertFalse(fed.can.contains(.office), "a Telegram-fed agent never writes files")
        let specs = AgentTools.specs(for: AgentTools.codexTools(for: maki))
        XCTAssertEqual(specs.compactMap { $0["name"].stringValue }.filter { OfficeTools.names.contains($0) }.sorted(), OfficeTools.names.sorted())
        XCTAssertTrue(CodexRequests.threadStart(id: 1, agent: maki, workspace: workspace).contains("create_presentation"))
    }
}
