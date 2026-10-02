import Foundation

/// The three Office tools an agent with the `office` capability is given (create_document, create_spreadsheet,
/// create_presentation): the JSON the agent writes becomes an .docx / .xlsx / .pptx in `documentos/` of its workspace
/// (OfficeFiles.swift). Nothing is ever written outside that folder, an existing file is never overwritten, and the sizes are capped.
enum OfficeTools {
    static let document = "create_document"
    static let spreadsheet = "create_spreadsheet"
    static let presentation = "create_presentation"
    static let names = [document, spreadsheet, presentation]
    static let folder = "documentos"

    static let maxBlocks = 500, maxItems = 200, maxText = 20_000
    static let maxSheets = 20, maxRows = 5000, maxColumns = 60
    static let maxSlides = 100, maxBullets = 30

    // MARK: specs

    private static func object(_ props: [String: JSONValue], required: [String]) -> JSONValue {
        .object(["type": .string("object"), "properties": .object(props), "required": .array(required.map { .string($0) }), "additionalProperties": .bool(false)])
    }
    private static let text = JSONValue.object(["type": .string("string")])
    private static func list(_ item: JSONValue) -> JSONValue { .object(["type": .string("array"), "items": item]) }

    static func specs(for names: [String]) -> [JSONValue] {
        names.compactMap { name -> JSONValue? in
            func spec(_ description: String, _ schema: JSONValue) -> JSONValue {
                .object(["type": .string("function"), "name": .string(name), "description": .string(description), "inputSchema": schema])
            }
            switch name {
            case document:
                let block = JSONValue.object(["type": .string("object"), "properties": .object([
                    "type": .object(["type": .string("string"), "enum": .array(["heading", "paragraph", "bullets", "numbered", "table", "pagebreak"].map { .string($0) })]),
                    "level": .object(["type": .string("integer"), "minimum": .number(1), "maximum": .number(3)]),
                    "text": text, "items": list(text), "header": list(text), "rows": list(list(text)),
                ]), "required": .array([.string("type")])])
                return spec("Create a Word document (.docx) in your documentos/ folder. `blocks` is the content in order: heading (level 1-3, text), paragraph (text; **bold**, *italic* and `code` work), bullets or numbered (items), table (header, rows) or pagebreak. Returns the file's path.",
                            object(["file": text, "title": text, "blocks": list(block)], required: ["file", "blocks"]))
            case spreadsheet:
                let cell = JSONValue.object(["type": .array([.string("string"), .string("number"), .string("boolean"), .string("null")])])
                let sheet = object(["name": text, "header": .object(["type": .string("boolean")]), "rows": list(list(cell))], required: ["rows"])
                return spec("Create an Excel workbook (.xlsx) in your documentos/ folder. Each sheet has `rows` (arrays of cells: text, numbers, booleans; a text starting with = is a formula, e.g. =SUM(B2:B9)). With header true (default) the first row is a bold, frozen, filtered header. Returns the file's path.",
                            object(["file": text, "sheets": list(sheet)], required: ["file", "sheets"]))
            case presentation:
                let slide = object(["title": text, "subtitle": text, "bullets": list(text)], required: ["title"])
                return spec("Create a PowerPoint presentation (.pptx) in your documentos/ folder. A first slide with no bullets is the cover (title, subtitle); every other slide is a title with bullets. Returns the file's path.",
                            object(["file": text, "slides": list(slide)], required: ["file", "slides"]))
            default: return nil
            }
        }
    }

    // MARK: running

    /// Writes the file the tool describes and returns where it is.
    static func run(_ tool: String, args: JSONValue, workspace: URL) throws -> URL {
        switch tool {
        case document:
            let blocks = try parseBlocks(args["blocks"])
            return try write(Zip.archive(DocxWriter.parts(title: clean(args["title"].stringValue), blocks: blocks)), name: args["file"].stringValue, ext: "docx", workspace: workspace)
        case spreadsheet:
            return try write(Zip.archive(XlsxWriter.parts(sheets: try parseSheets(args["sheets"]))), name: args["file"].stringValue, ext: "xlsx", workspace: workspace)
        case presentation:
            return try write(Zip.archive(PptxWriter.parts(slides: try parseSlides(args["slides"]))), name: args["file"].stringValue, ext: "pptx", workspace: workspace)
        default:
            throw OfficeError("Herramienta desconocida.")
        }
    }

    // MARK: files

    /// A file name that stays in the folder: no directories, a safe set of characters, the right extension, at most 80 characters.
    static func fileName(_ raw: String?, ext: String) throws -> String {
        let base = (raw ?? "").replacingOccurrences(of: "\\", with: "/").split(separator: "/").last.map(String.init) ?? ""
        var stem = base
        if let dot = base.lastIndex(of: "."), base[base.index(after: dot)...].count <= 5 { stem = String(base[..<dot]) }
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: " _-()"))
        let cleaned = String(stem.unicodeScalars.map { allowed.contains($0) ? Character($0) : "_" }).trimmingCharacters(in: CharacterSet(charactersIn: " ._"))
        guard !cleaned.isEmpty else { throw OfficeError("Falta el nombre del archivo.") }
        return String(cleaned.prefix(80)) + "." + ext
    }

    private static func write(_ data: Data, name: String?, ext: String, workspace: URL) throws -> URL {
        let wanted = try fileName(name, ext: ext)
        let dir = workspace.appendingPathComponent(folder, isDirectory: true)
        do { try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700]) }
        catch { throw OfficeError("No se pudo crear la carpeta documentos.") }
        // Never over a file that exists: "informe.docx" becomes "informe (2).docx".
        var url = dir.appendingPathComponent(wanted)
        var n = 2
        let stem = (wanted as NSString).deletingPathExtension
        while FileManager.default.fileExists(atPath: url.path), n < 1000 {
            url = dir.appendingPathComponent("\(stem) (\(n)).\(ext)"); n += 1
        }
        do { try data.write(to: url, options: .atomic) } catch { throw OfficeError("No se pudo guardar el archivo.") }
        return url
    }

    // MARK: parsing

    private static func clean(_ text: String?) -> String? {
        guard let text = text?.trimmingCharacters(in: .whitespacesAndNewlines), !text.isEmpty else { return nil }
        return String(text.prefix(maxText))
    }

    private static func strings(_ value: JSONValue, limit: Int, what: String) throws -> [String] {
        guard case .array(let items) = value else { return [] }
        guard items.count <= limit else { throw OfficeError("Demasiados elementos en \(what) (máximo \(limit)).") }
        return items.map { String(cellText($0).prefix(maxText)) }
    }

    private static func cellText(_ value: JSONValue) -> String {
        switch value {
        case .string(let s): return s
        case .number(let n): return n == n.rounded() && abs(n) < 1e15 ? String(Int64(n)) : String(n)
        case .bool(let b): return b ? "Sí" : "No"
        default: return ""
        }
    }

    static func parseBlocks(_ value: JSONValue) throws -> [DocBlock] {
        guard case .array(let raw) = value, !raw.isEmpty else { throw OfficeError("El documento no tiene contenido (blocks).") }
        guard raw.count <= maxBlocks else { throw OfficeError("Demasiados bloques (máximo \(maxBlocks)).") }
        return try raw.map { block in
            switch block["type"].stringValue {
            case "heading": return .heading(level: block["level"].intValue ?? 1, text: String((block["text"].stringValue ?? "").prefix(maxText)))
            case "paragraph": return .paragraph(String((block["text"].stringValue ?? "").prefix(maxText)))
            case "bullets": return .bullets(try strings(block["items"], limit: maxItems, what: "la lista"))
            case "numbered": return .numbered(try strings(block["items"], limit: maxItems, what: "la lista"))
            case "table":
                let header = try strings(block["header"], limit: maxColumns, what: "el encabezado")
                guard case .array(let rows) = block["rows"] else { return .table(header: header, rows: []) }
                guard rows.count <= maxRows else { throw OfficeError("Demasiadas filas en la tabla (máximo \(maxRows)).") }
                return .table(header: header, rows: try rows.map { try strings($0, limit: maxColumns, what: "una fila") })
            case "pagebreak": return .pageBreak
            default: throw OfficeError("Tipo de bloque desconocido. Usa heading, paragraph, bullets, numbered, table o pagebreak.")
            }
        }
    }

    static func parseSheets(_ value: JSONValue) throws -> [SheetData] {
        guard case .array(let raw) = value, !raw.isEmpty else { throw OfficeError("El libro no tiene hojas (sheets).") }
        guard raw.count <= maxSheets else { throw OfficeError("Demasiadas hojas (máximo \(maxSheets)).") }
        return try raw.enumerated().map { index, sheet in
            guard case .array(let rows) = sheet["rows"], rows.count <= maxRows else { throw OfficeError("Cada hoja necesita `rows` (máximo \(maxRows) filas).") }
            let cells: [[SheetCell]] = try rows.map { row in
                guard case .array(let items) = row, items.count <= maxColumns else { throw OfficeError("Cada fila es una lista de celdas (máximo \(maxColumns)).") }
                return items.map { item in
                    switch item {
                    case .string(let s): return s.hasPrefix("=") && s.count > 1 ? .formula(String(s.dropFirst())) : .text(String(s.prefix(maxText)))
                    case .number(let n): return n.isFinite ? .number(n) : .empty
                    case .bool(let b): return .bool(b)
                    default: return .empty
                    }
                }
            }
            return SheetData(name: XlsxWriter.sheetName(sheet["name"].stringValue ?? "", index: index), rows: cells, header: sheet["header"].boolValue ?? true)
        }
    }

    static func parseSlides(_ value: JSONValue) throws -> [SlideData] {
        guard case .array(let raw) = value, !raw.isEmpty else { throw OfficeError("La presentación no tiene diapositivas (slides).") }
        guard raw.count <= maxSlides else { throw OfficeError("Demasiadas diapositivas (máximo \(maxSlides)).") }
        return try raw.map { slide in
            SlideData(title: String((slide["title"].stringValue ?? "").prefix(300)), subtitle: slide["subtitle"].stringValue.map { String($0.prefix(300)) },
                      bullets: try strings(slide["bullets"], limit: maxBullets, what: "las viñetas"))
        }
    }
}
