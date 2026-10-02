import Foundation

// Word, Excel and PowerPoint files, written by MIKA itself: an .docx / .xlsx / .pptx is a ZIP of XML files, so no library, no
// network and no other program is involved. The agent describes the content as JSON (headings, paragraphs, lists, tables; sheets
// of rows; slides of bullets) and MIKA writes the file in the agent's own workspace. Only agents with the `office` capability get
// these tools (MAKI and MIRA by default), and they only ever write inside their workspace.

struct OfficeError: Error, Equatable { var message: String; init(_ message: String) { self.message = message } }

// MARK: - ZIP (stored, no compression)

enum Zip {
    private static let table: [UInt32] = (0..<256).map { i -> UInt32 in
        var c = UInt32(i)
        for _ in 0..<8 { c = c & 1 == 1 ? 0xEDB8_8320 ^ (c >> 1) : c >> 1 }
        return c
    }

    static func crc32(_ data: Data) -> UInt32 {
        var crc: UInt32 = 0xFFFF_FFFF
        for byte in data { crc = table[Int((crc ^ UInt32(byte)) & 0xFF)] ^ (crc >> 8) }
        return crc ^ 0xFFFF_FFFF
    }

    /// A ZIP of `entries` (path, bytes), in order, each stored as is. Paths use `/`. 2026-01-01, so the same input is the same file.
    static func archive(_ entries: [(path: String, data: Data)]) -> Data {
        var out = Data()
        var central = Data()
        func u16(_ v: Int, into d: inout Data) { d.append(UInt8(v & 0xFF)); d.append(UInt8((v >> 8) & 0xFF)) }
        func u32(_ v: UInt32, into d: inout Data) { for shift in stride(from: 0, to: 32, by: 8) { d.append(UInt8((v >> UInt32(shift)) & 0xFF)) } }
        let dosDate = ((2026 - 1980) << 9) | (1 << 5) | 1
        for entry in entries {
            let name = Data(entry.path.utf8)
            let crc = crc32(entry.data)
            let offset = UInt32(out.count)
            u32(0x0403_4B50, into: &out); u16(20, into: &out); u16(0x0800, into: &out); u16(0, into: &out)
            u16(0, into: &out); u16(dosDate, into: &out)
            u32(crc, into: &out); u32(UInt32(entry.data.count), into: &out); u32(UInt32(entry.data.count), into: &out)
            u16(name.count, into: &out); u16(0, into: &out)
            out.append(name); out.append(entry.data)

            u32(0x0201_4B50, into: &central); u16(20, into: &central); u16(20, into: &central); u16(0x0800, into: &central); u16(0, into: &central)
            u16(0, into: &central); u16(dosDate, into: &central)
            u32(crc, into: &central); u32(UInt32(entry.data.count), into: &central); u32(UInt32(entry.data.count), into: &central)
            u16(name.count, into: &central); u16(0, into: &central); u16(0, into: &central); u16(0, into: &central); u16(0, into: &central)
            u32(0, into: &central); u32(offset, into: &central)
            central.append(name)
        }
        let centralOffset = UInt32(out.count)
        out.append(central)
        u32(0x0605_4B50, into: &out); u16(0, into: &out); u16(0, into: &out)
        u16(entries.count, into: &out); u16(entries.count, into: &out)
        u32(UInt32(central.count), into: &out); u32(centralOffset, into: &out); u16(0, into: &out)
        return out
    }
}

// MARK: - XML helpers

enum OfficeXML {
    /// Text safe for an XML node: the five special characters escaped and control characters dropped.
    static func escape(_ text: String) -> String {
        var out = ""
        for scalar in text.unicodeScalars {
            switch scalar {
            case "&": out += "&amp;"
            case "<": out += "&lt;"
            case ">": out += "&gt;"
            case "\"": out += "&quot;"
            case "\t", "\n", "\r": out.unicodeScalars.append(scalar)
            default: if scalar.value >= 0x20, !(0xD800...0xDFFF).contains(scalar.value), scalar.value != 0xFFFE, scalar.value != 0xFFFF { out.unicodeScalars.append(scalar) }
            }
        }
        return out
    }

    static let header = #"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"# + "\n"

    static func part(_ path: String, _ xml: String) -> (path: String, data: Data) { (path, Data((header + xml).utf8)) }

    /// A1-style column name: 0 → "A", 26 → "AA".
    static func column(_ index: Int) -> String {
        var n = index + 1
        var out = ""
        while n > 0 { let r = (n - 1) % 26; out = String(UnicodeScalar(UInt8(65 + r))) + out; n = (n - 1) / 26 }
        return out
    }

    /// `**bold**`, `*italic*` and `` `code` `` in a line of text, as runs.
    struct Run: Equatable { var text: String; var bold = false; var italic = false; var code = false }

    static func runs(_ source: String) -> [Run] {
        let chars = Array(source)
        var out: [Run] = []
        var buffer = ""
        var bold = false, italic = false
        func flush() { if !buffer.isEmpty { out.append(Run(text: buffer, bold: bold, italic: italic)); buffer = "" } }
        var i = 0
        while i < chars.count {
            if chars[i] == "`", let end = chars[(i + 1)...].firstIndex(of: "`"), end > i + 1 {
                flush(); out.append(Run(text: String(chars[(i + 1)..<end]), code: true)); i = end + 1; continue
            }
            if chars[i] == "*", i + 1 < chars.count, chars[i + 1] == "*" { flush(); bold.toggle(); i += 2; continue }
            if chars[i] == "*", i + 1 < chars.count, chars[i + 1] != " " || italic { flush(); italic.toggle(); i += 1; continue }
            buffer.append(chars[i]); i += 1
        }
        flush()
        return out
    }
}

// MARK: - Word

enum DocBlock: Equatable {
    case heading(level: Int, text: String)
    case paragraph(String)
    case bullets([String])
    case numbered([String])
    case table(header: [String], rows: [[String]])
    case pageBreak
}

enum DocxWriter {
    static let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"

    private static func runXML(_ run: OfficeXML.Run) -> String {
        var props = ""
        if run.code { props += #"<w:rFonts w:ascii="Consolas" w:hAnsi="Consolas" w:cs="Consolas"/><w:shd w:val="clear" w:color="auto" w:fill="F2F2F2"/>"# }
        if run.bold { props += "<w:b/>" }
        if run.italic { props += "<w:i/>" }
        return "<w:r>" + (props.isEmpty ? "" : "<w:rPr>\(props)</w:rPr>") + "<w:t xml:space=\"preserve\">\(OfficeXML.escape(run.text))</w:t></w:r>"
    }

    private static func paragraph(_ text: String, style: String? = nil, numId: Int? = nil, bold: Bool = false, extra: String = "") -> String {
        var props = ""
        if let style { props += "<w:pStyle w:val=\"\(style)\"/>" }
        if let numId { props += "<w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"\(numId)\"/></w:numPr>" }
        props += extra
        let runs = OfficeXML.runs(text).map { var r = $0; r.bold = r.bold || bold; return runXML(r) }.joined()
        return "<w:p>" + (props.isEmpty ? "" : "<w:pPr>\(props)</w:pPr>") + runs + "</w:p>"
    }

    private static func table(header: [String], rows: [[String]]) -> String {
        let columns = max(header.count, rows.map(\.count).max() ?? 0, 1)
        let width = 9026 / columns
        func cell(_ text: String, head: Bool) -> String {
            "<w:tc><w:tcPr><w:tcW w:w=\"\(width)\" w:type=\"dxa\"/>" + (head ? #"<w:shd w:val="clear" w:color="auto" w:fill="D9E2F3"/>"# : "")
                + "</w:tcPr>" + paragraph(text, bold: head, extra: #"<w:spacing w:after="0"/>"#) + "</w:tc>"
        }
        func row(_ cells: [String], head: Bool) -> String {
            "<w:tr>" + (head ? "<w:trPr><w:tblHeader/></w:trPr>" : "")
                + (0..<columns).map { cell($0 < cells.count ? cells[$0] : "", head: head) }.joined() + "</w:tr>"
        }
        var xml = #"<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="5000" w:type="pct"/></w:tblPr><w:tblGrid>"#
        xml += String(repeating: "<w:gridCol w:w=\"\(width)\"/>", count: columns) + "</w:tblGrid>"
        if !header.isEmpty { xml += row(header, head: true) }
        xml += rows.map { row($0, head: false) }.joined()
        return xml + "</w:tbl><w:p/>"
    }

    /// The parts of a .docx.
    static func parts(title: String?, blocks: [DocBlock]) -> [(path: String, data: Data)] {
        var body = ""
        var numberedLists = 0
        if let title, !title.isEmpty { body += paragraph(title, style: "Title") }
        for block in blocks {
            switch block {
            case .heading(let level, let text): body += paragraph(text, style: "Heading\(min(max(level, 1), 3))")
            case .paragraph(let text): body += paragraph(text)
            case .bullets(let items): body += items.map { paragraph($0, style: "ListParagraph", numId: 1) }.joined()
            case .numbered(let items):
                numberedLists += 1
                body += items.map { paragraph($0, style: "ListParagraph", numId: 1 + numberedLists) }.joined()
            case .table(let header, let rows): body += table(header: header, rows: rows)
            case .pageBreak: body += #"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#
            }
        }
        let document = "<w:document xmlns:w=\"\(w)\"><w:body>\(body)<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"708\" w:footer=\"708\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>"

        var nums = #"<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#
        for list in 0..<numberedLists {
            nums += "<w:num w:numId=\"\(list + 2)\"><w:abstractNumId w:val=\"1\"/><w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"1\"/></w:lvlOverride></w:num>"
        }
        let numbering = "<w:numbering xmlns:w=\"\(w)\">"
            + #"<w:abstractNum w:abstractNumId="0"><w:multiLevelType w:val="hybridMultilevel"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="•"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum>"#
            + #"<w:abstractNum w:abstractNumId="1"><w:multiLevelType w:val="hybridMultilevel"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum>"#
            + nums + "</w:numbering>"

        func heading(_ id: String, _ name: String, _ size: Int, _ level: Int) -> String {
            "<w:style w:type=\"paragraph\" w:styleId=\"\(id)\"><w:name w:val=\"\(name)\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before=\"320\" w:after=\"120\"/><w:outlineLvl w:val=\"\(level)\"/></w:pPr><w:rPr><w:b/><w:color w:val=\"1F3864\"/><w:sz w:val=\"\(size)\"/><w:szCs w:val=\"\(size)\"/></w:rPr></w:style>"
        }
        let styles = "<w:styles xmlns:w=\"\(w)\">"
            + #"<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="Calibri" w:cs="Calibri"/><w:sz w:val="22"/><w:szCs w:val="22"/><w:lang w:val="es-PE"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="120" w:line="276" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults>"#
            + #"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>"#
            + #"<w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:spacing w:after="240"/></w:pPr><w:rPr><w:b/><w:color w:val="1F3864"/><w:sz w:val="48"/><w:szCs w:val="48"/></w:rPr></w:style>"#
            + heading("Heading1", "heading 1", 32, 0) + heading("Heading2", "heading 2", 28, 1) + heading("Heading3", "heading 3", 24, 2)
            + #"<w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:spacing w:after="60"/><w:ind w:left="720"/></w:pPr></w:style>"#
            + #"<w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/><w:uiPriority w:val="99"/><w:semiHidden/><w:tblPr><w:tblInd w:w="0" w:type="dxa"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>"#
            + #"<w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/><w:basedOn w:val="TableNormal"/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/><w:left w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/><w:bottom w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/><w:right w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/><w:insideH w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/><w:insideV w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/></w:tblBorders></w:tblPr></w:style>"#
            + "</w:styles>"

        let types = #"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/></Types>"#
        let rels = #"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#
        let documentRels = #"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/></Relationships>"#
        return [OfficeXML.part("[Content_Types].xml", types), OfficeXML.part("_rels/.rels", rels), OfficeXML.part("word/document.xml", document),
                OfficeXML.part("word/_rels/document.xml.rels", documentRels), OfficeXML.part("word/styles.xml", styles),
                OfficeXML.part("word/numbering.xml", numbering)]
    }
}

// MARK: - Excel

enum SheetCell: Equatable {
    case text(String), number(Double), bool(Bool), formula(String), empty
}

struct SheetData: Equatable {
    var name: String
    var rows: [[SheetCell]]
    /// The first row is a header: bold on a band, frozen, with a filter.
    var header = true
}

enum XlsxWriter {
    static let main = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
    static let rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"

    /// A sheet name Excel accepts: at most 31 characters, none of `[]:*?/\`, not empty.
    static func sheetName(_ raw: String, index: Int) -> String {
        let cleaned = String(raw.unicodeScalars.map { "[]:*?/\\".unicodeScalars.contains($0) ? "_" : Character($0) })
            .trimmingCharacters(in: CharacterSet(charactersIn: "' ").union(.whitespaces))
        return String((cleaned.isEmpty ? "Hoja\(index + 1)" : cleaned).prefix(31))
    }

    private static func sheetXML(_ sheet: SheetData) -> String {
        let columns = max(sheet.rows.map(\.count).max() ?? 1, 1)
        var widths = [Int](repeating: 8, count: columns)
        for row in sheet.rows {
            for (c, cell) in row.enumerated() where c < columns {
                let length: Int
                switch cell {
                case .text(let t): length = t.count + 2
                case .number(let n): length = String(n).count + 2
                case .bool: length = 7
                case .formula: length = 12
                case .empty: length = 0
                }
                widths[c] = min(60, max(widths[c], length))
            }
        }
        var xml = "<worksheet xmlns=\"\(main)\"><sheetViews><sheetView workbookViewId=\"0\">"
        if sheet.header, sheet.rows.count > 1 { xml += #"<pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/>"# }
        xml += "</sheetView></sheetViews><sheetFormatPr defaultRowHeight=\"15\"/><cols>"
        for (c, width) in widths.enumerated() { xml += "<col min=\"\(c + 1)\" max=\"\(c + 1)\" width=\"\(width)\" customWidth=\"1\"/>" }
        xml += "</cols><sheetData>"
        for (r, row) in sheet.rows.enumerated() {
            xml += "<row r=\"\(r + 1)\">"
            for (c, cell) in row.enumerated() {
                let ref = OfficeXML.column(c) + String(r + 1)
                let style = sheet.header && r == 0 ? " s=\"1\"" : ""
                switch cell {
                case .empty: if !style.isEmpty { xml += "<c r=\"\(ref)\"\(style)/>" }
                case .text(let t): xml += "<c r=\"\(ref)\"\(style) t=\"inlineStr\"><is><t xml:space=\"preserve\">\(OfficeXML.escape(t))</t></is></c>"
                case .number(let n): xml += "<c r=\"\(ref)\"\(style)><v>\(n == n.rounded() && abs(n) < 1e15 ? String(Int64(n)) : String(n))</v></c>"
                case .bool(let b): xml += "<c r=\"\(ref)\"\(style) t=\"b\"><v>\(b ? 1 : 0)</v></c>"
                case .formula(let f): xml += "<c r=\"\(ref)\"\(style)><f>\(OfficeXML.escape(f))</f></c>"
                }
            }
            xml += "</row>"
        }
        xml += "</sheetData>"
        if sheet.header, sheet.rows.count > 1 { xml += "<autoFilter ref=\"A1:\(OfficeXML.column(columns - 1))\(sheet.rows.count)\"/>" }
        return xml + "</worksheet>"
    }

    static func parts(sheets: [SheetData]) -> [(path: String, data: Data)] {
        let count = sheets.count
        var types = #"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#
        for i in 1...count { types += "<Override PartName=\"/xl/worksheets/sheet\(i).xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>" }
        types += "</Types>"
        let rels = #"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
        var sheetList = ""
        var workbookRels = #"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#
        for (i, sheet) in sheets.enumerated() {
            sheetList += "<sheet name=\"\(OfficeXML.escape(sheet.name))\" sheetId=\"\(i + 1)\" r:id=\"rId\(i + 1)\"/>"
            workbookRels += "<Relationship Id=\"rId\(i + 1)\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet\(i + 1).xml\"/>"
        }
        workbookRels += "<Relationship Id=\"rId\(count + 1)\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>"
        let workbook = "<workbook xmlns=\"\(main)\" xmlns:r=\"\(rel)\"><sheets>\(sheetList)</sheets></workbook>"
        let styles = "<styleSheet xmlns=\"\(main)\">"
            + #"<fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts>"#
            + #"<fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFD9E2F3"/><bgColor indexed="64"/></patternFill></fill></fills>"#
            + #"<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>"#
            + #"<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>"#
            + #"<cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="2" borderId="0" xfId="0" applyFont="1" applyFill="1"/></cellXfs>"#
            + #"<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#
        var out = [OfficeXML.part("[Content_Types].xml", types), OfficeXML.part("_rels/.rels", rels), OfficeXML.part("xl/workbook.xml", workbook),
                   OfficeXML.part("xl/_rels/workbook.xml.rels", workbookRels), OfficeXML.part("xl/styles.xml", styles)]
        for (i, sheet) in sheets.enumerated() { out.append(OfficeXML.part("xl/worksheets/sheet\(i + 1).xml", sheetXML(sheet))) }
        return out
    }
}

// MARK: - PowerPoint

struct SlideData: Equatable {
    var title: String
    var subtitle: String?
    var bullets: [String]
}

enum PptxWriter {
    static let a = "http://schemas.openxmlformats.org/drawingml/2006/main"
    static let r = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
    static let p = "http://schemas.openxmlformats.org/presentationml/2006/main"
    private static let ns = "xmlns:a=\"\(a)\" xmlns:r=\"\(r)\" xmlns:p=\"\(p)\""

    private static func paragraphs(_ lines: [String]) -> String {
        lines.map { line in
            "<a:p>" + OfficeXML.runs(line).map { run in
                "<a:r><a:rPr lang=\"es-PE\"\(run.bold ? " b=\"1\"" : "")\(run.italic ? " i=\"1\"" : "")/><a:t>\(OfficeXML.escape(run.text))</a:t></a:r>"
            }.joined() + "</a:p>"
        }.joined()
    }

    private static func placeholder(id: Int, name: String, type: String?, idx: Int?, lines: [String]) -> String {
        var ph = "<p:ph"
        if let type { ph += " type=\"\(type)\"" }
        if let idx { ph += " idx=\"\(idx)\"" }
        ph += "/>"
        let body = lines.isEmpty ? "<a:p><a:endParaRPr lang=\"es-PE\"/></a:p>" : paragraphs(lines)
        return "<p:sp><p:nvSpPr><p:cNvPr id=\"\(id)\" name=\"\(name)\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr>\(ph)</p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/>\(body)</p:txBody></p:sp>"
    }

    private static func slideXML(_ slide: SlideData, cover: Bool) -> String {
        var shapes = placeholder(id: 2, name: "Título 1", type: cover ? "ctrTitle" : "title", idx: nil, lines: [slide.title])
        if cover {
            if let subtitle = slide.subtitle, !subtitle.isEmpty { shapes += placeholder(id: 3, name: "Subtítulo 2", type: "subTitle", idx: 1, lines: [subtitle]) }
        } else {
            shapes += placeholder(id: 3, name: "Contenido 2", type: nil, idx: 1, lines: slide.bullets)
        }
        return "<p:sld \(ns)><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>\(shapes)</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"
    }

    private static func shape(id: Int, name: String, ph: String, x: Int, y: Int, cx: Int, cy: Int, anchor: String, size: Int, bold: Bool, align: String, sample: String) -> String {
        "<p:sp><p:nvSpPr><p:cNvPr id=\"\(id)\" name=\"\(name)\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr>\(ph)</p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"\(x)\" y=\"\(y)\"/><a:ext cx=\"\(cx)\" cy=\"\(cy)\"/></a:xfrm></p:spPr><p:txBody><a:bodyPr anchor=\"\(anchor)\"><a:normAutofit/></a:bodyPr><a:lstStyle><a:lvl1pPr algn=\"\(align)\"><a:defRPr sz=\"\(size)\"\(bold ? " b=\"1\"" : "")/></a:lvl1pPr></a:lstStyle><a:p><a:r><a:rPr lang=\"es-PE\"/><a:t>\(sample)</a:t></a:r></a:p></p:txBody></p:sp>"
    }

    private static let group = "<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>"

    private static func levelStyle(_ tag: String, size: Int, bullet: Bool, margin: Int = 0) -> String {
        "<a:\(tag) marL=\"\(bullet ? 342900 + margin : 0)\" indent=\"\(bullet ? -342900 : 0)\" algn=\"l\"><a:spcBef><a:spcPts val=\"600\"/></a:spcBef>"
            + (bullet ? "<a:buFont typeface=\"Arial\"/><a:buChar char=\"•\"/>" : "<a:buNone/>")
            + "<a:defRPr sz=\"\(size)\" kern=\"1200\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mn-lt\"/></a:defRPr></a:\(tag)>"
    }

    static func parts(slides: [SlideData]) -> [(path: String, data: Data)] {
        let count = slides.count
        var types = #"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/><Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/><Override PartName="/ppt/slideLayouts/slideLayout2.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/><Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>"#
        for i in 1...count { types += "<Override PartName=\"/ppt/slides/slide\(i).xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>" }
        types += "</Types>"
        let rels = #"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#

        var ids = ""
        var presentationRels = #"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>"#
        for i in 1...count {
            ids += "<p:sldId id=\"\(255 + i)\" r:id=\"rId\(i + 1)\"/>"
            presentationRels += "<Relationship Id=\"rId\(i + 1)\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide\(i).xml\"/>"
        }
        presentationRels += "<Relationship Id=\"rId\(count + 2)\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"theme/theme1.xml\"/></Relationships>"
        let presentation = "<p:presentation \(ns)><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst><p:sldIdLst>\(ids)</p:sldIdLst><p:sldSz cx=\"12192000\" cy=\"6858000\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>"

        let master = "<p:sldMaster \(ns)><p:cSld><p:bg><p:bgRef idx=\"1001\"><a:schemeClr val=\"bg1\"/></p:bgRef></p:bg><p:spTree>\(group)"
            + shape(id: 2, name: "Título 1", ph: "<p:ph type=\"title\"/>", x: 838200, y: 365125, cx: 10515600, cy: 1325563, anchor: "ctr", size: 4400, bold: true, align: "l", sample: "Título")
            + shape(id: 3, name: "Texto 2", ph: "<p:ph type=\"body\" idx=\"1\"/>", x: 838200, y: 1825625, cx: 10515600, cy: 4351338, anchor: "t", size: 2800, bold: false, align: "l", sample: "Texto")
            + "</p:spTree></p:cSld><p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>"
            + "<p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/><p:sldLayoutId id=\"2147483650\" r:id=\"rId2\"/></p:sldLayoutIdLst>"
            + "<p:txStyles><p:titleStyle>" + levelStyle("lvl1pPr", size: 4400, bullet: false) + "</p:titleStyle><p:bodyStyle>"
            + (1...5).map { levelStyle("lvl\($0)pPr", size: max(1800, 2800 - ($0 - 1) * 400), bullet: true, margin: ($0 - 1) * 342900) }.joined()
            + "</p:bodyStyle><p:otherStyle>" + levelStyle("lvl1pPr", size: 1800, bullet: false) + "</p:otherStyle></p:txStyles></p:sldMaster>"
        let masterRels = #"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout2.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/></Relationships>"#

        let coverLayout = "<p:sldLayout \(ns) type=\"title\" preserve=\"1\"><p:cSld name=\"Portada\"><p:spTree>\(group)"
            + shape(id: 2, name: "Título 1", ph: "<p:ph type=\"ctrTitle\"/>", x: 1524000, y: 1122363, cx: 9144000, cy: 2387600, anchor: "b", size: 6000, bold: true, align: "ctr", sample: "Título")
            + shape(id: 3, name: "Subtítulo 2", ph: "<p:ph type=\"subTitle\" idx=\"1\"/>", x: 1524000, y: 3602038, cx: 9144000, cy: 1655762, anchor: "t", size: 2400, bold: false, align: "ctr", sample: "Subtítulo")
            + "</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"
        let contentLayout = "<p:sldLayout \(ns) type=\"obj\" preserve=\"1\"><p:cSld name=\"Título y contenido\"><p:spTree>\(group)"
            + "<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Título 1\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr><p:ph type=\"title\"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang=\"es-PE\"/><a:t>Título</a:t></a:r></a:p></p:txBody></p:sp>"
            + "<p:sp><p:nvSpPr><p:cNvPr id=\"3\" name=\"Contenido 2\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr><p:ph idx=\"1\"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang=\"es-PE\"/><a:t>Texto</a:t></a:r></a:p></p:txBody></p:sp>"
            + "</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"
        let layoutRels = #"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/></Relationships>"#

        func fills(_ n: Int) -> String { String(repeating: "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>", count: n) }
        let theme = "<a:theme xmlns:a=\"\(a)\" name=\"MIKA\"><a:themeElements><a:clrScheme name=\"MIKA\">"
            + #"<a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="1F2937"/></a:dk2><a:lt2><a:srgbClr val="F3F4F6"/></a:lt2>"#
            + #"<a:accent1><a:srgbClr val="2F5597"/></a:accent1><a:accent2><a:srgbClr val="ED7D31"/></a:accent2><a:accent3><a:srgbClr val="70AD47"/></a:accent3><a:accent4><a:srgbClr val="FFC000"/></a:accent4><a:accent5><a:srgbClr val="5B9BD5"/></a:accent5><a:accent6><a:srgbClr val="7F7F7F"/></a:accent6><a:hlink><a:srgbClr val="0563C1"/></a:hlink><a:folHlink><a:srgbClr val="954F72"/></a:folHlink>"#
            + #"</a:clrScheme><a:fontScheme name="MIKA"><a:majorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme>"#
            + "<a:fmtScheme name=\"MIKA\"><a:fillStyleLst>\(fills(3))</a:fillStyleLst>"
            + "<a:lnStyleLst>" + String(repeating: #"<a:ln w="9525"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>"#, count: 3) + "</a:lnStyleLst>"
            + "<a:effectStyleLst>" + String(repeating: "<a:effectStyle><a:effectLst/></a:effectStyle>", count: 3) + "</a:effectStyleLst>"
            + "<a:bgFillStyleLst>\(fills(3))</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"

        var out = [OfficeXML.part("[Content_Types].xml", types), OfficeXML.part("_rels/.rels", rels), OfficeXML.part("ppt/presentation.xml", presentation),
                   OfficeXML.part("ppt/_rels/presentation.xml.rels", presentationRels), OfficeXML.part("ppt/slideMasters/slideMaster1.xml", master),
                   OfficeXML.part("ppt/slideMasters/_rels/slideMaster1.xml.rels", masterRels),
                   OfficeXML.part("ppt/slideLayouts/slideLayout1.xml", coverLayout), OfficeXML.part("ppt/slideLayouts/slideLayout2.xml", contentLayout),
                   OfficeXML.part("ppt/slideLayouts/_rels/slideLayout1.xml.rels", layoutRels), OfficeXML.part("ppt/slideLayouts/_rels/slideLayout2.xml.rels", layoutRels),
                   OfficeXML.part("ppt/theme/theme1.xml", theme)]
        for (i, slide) in slides.enumerated() {
            let cover = i == 0 && slide.bullets.isEmpty
            out.append(OfficeXML.part("ppt/slides/slide\(i + 1).xml", slideXML(slide, cover: cover)))
            let layout = cover ? 1 : 2
            out.append(OfficeXML.part("ppt/slides/_rels/slide\(i + 1).xml.rels", "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout\(layout).xml\"/></Relationships>"))
        }
        return out
    }
}
