// Ported from MIKA (MIT, revision d050bc5): apps/macos/Sources/Providers/OfficeTools.swift
//! The three Office tools Buddy's agents get over MCP (`create_document`, `create_spreadsheet`,
//! `create_presentation`): the JSON the agent writes becomes a .docx / .xlsx / .pptx in the documents folder
//! `buddy-hook --mcp --out <dir>` was started with. An Office file is a ZIP of XML, so no library, no network and
//! no other program is involved.
//!
//! Nothing is ever written outside that folder, an existing file is never overwritten (`informe.docx` becomes
//! `informe (2).docx`), and every size is capped.

mod docx;
mod pptx;
mod xlsx;
mod xml;
mod zip;

use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

pub use docx::DocBlock;
pub use pptx::SlideData;
pub use xlsx::{SheetCell, SheetData};

pub const DOCUMENT: &str = "create_document";
pub const SPREADSHEET: &str = "create_spreadsheet";
pub const PRESENTATION: &str = "create_presentation";
pub const NAMES: [&str; 3] = [DOCUMENT, SPREADSHEET, PRESENTATION];

pub const MAX_BLOCKS: usize = 500;
pub const MAX_ITEMS: usize = 200;
pub const MAX_TEXT: usize = 20_000;
pub const MAX_SHEETS: usize = 20;
pub const MAX_ROWS: usize = 5000;
pub const MAX_COLUMNS: usize = 60;
pub const MAX_SLIDES: usize = 100;
pub const MAX_BULLETS: usize = 30;
/// Longest slide title or subtitle.
const MAX_SLIDE_TITLE: usize = 300;
/// Longest file name, before the extension.
const MAX_FILE_NAME: usize = 80;
/// How many "name (n)" variants are tried before giving up.
const MAX_VARIANTS: usize = 1000;

/// Why a tool did not write its file, in words the agent can pass on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfficeError(pub String);

impl OfficeError {
    pub fn new(message: impl Into<String>) -> Self {
        OfficeError(message.into())
    }
}

impl std::fmt::Display for OfficeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// MARK: specs

fn object(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required, "additionalProperties": false })
}

fn list(item: Value, max: usize) -> Value {
    json!({ "type": "array", "items": item, "maxItems": max })
}

/// The MCP `tools/list` entries: name, title, Spanish description and JSON Schema input.
pub fn specs() -> Vec<Value> {
    let text = json!({ "type": "string" });
    let file = json!({ "type": "string", "description": "Nombre del archivo, sin carpetas (la extensión la pone la herramienta)." });
    let annotations = json!({ "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false });

    let block = json!({
        "type": "object",
        "properties": {
            "type": { "type": "string", "enum": ["heading", "paragraph", "bullets", "numbered", "table", "pagebreak"] },
            "level": { "type": "integer", "minimum": 1, "maximum": 3 },
            "text": text,
            "items": list(text.clone(), MAX_ITEMS),
            "header": list(text.clone(), MAX_COLUMNS),
            "rows": list(list(text.clone(), MAX_COLUMNS), MAX_ROWS),
        },
        "required": ["type"],
    });
    let cell = json!({ "type": ["string", "number", "boolean", "null"] });
    let sheet = object(
        json!({ "name": text, "header": { "type": "boolean" }, "rows": list(list(cell, MAX_COLUMNS), MAX_ROWS) }),
        &["rows"],
    );
    let slide = object(json!({ "title": text, "subtitle": text, "bullets": list(text.clone(), MAX_BULLETS) }), &["title"]);

    vec![
        json!({
            "name": DOCUMENT,
            "title": "Crear documento de Word",
            "description": "Crea un documento de Word (.docx) en la carpeta de documentos de Buddy. `blocks` es el contenido en orden: heading (level 1-3, text), paragraph (text; **negrita**, *cursiva* y `código` funcionan), bullets o numbered (items), table (header, rows) o pagebreak. Si ya existe un archivo con ese nombre se crea «nombre (2)». Devuelve la ruta del archivo creado.",
            "inputSchema": object(json!({ "file": file, "title": text, "blocks": list(block, MAX_BLOCKS) }), &["file", "blocks"]),
            "annotations": annotations,
        }),
        json!({
            "name": SPREADSHEET,
            "title": "Crear libro de Excel",
            "description": "Crea un libro de Excel (.xlsx) en la carpeta de documentos de Buddy. Cada hoja tiene `rows` (listas de celdas: texto, números, booleanos; un texto que empieza con = es una fórmula, p. ej. =SUM(B2:B9)). Con header true (por defecto) la primera fila es un encabezado en negrita, fijo y con filtro. Si ya existe un archivo con ese nombre se crea «nombre (2)». Devuelve la ruta del archivo creado.",
            "inputSchema": object(json!({ "file": file, "sheets": list(sheet, MAX_SHEETS) }), &["file", "sheets"]),
            "annotations": annotations,
        }),
        json!({
            "name": PRESENTATION,
            "title": "Crear presentación de PowerPoint",
            "description": "Crea una presentación de PowerPoint (.pptx) en la carpeta de documentos de Buddy. Si la primera diapositiva no tiene viñetas es la portada (title, subtitle); las demás son un título con viñetas (bullets). Si ya existe un archivo con ese nombre se crea «nombre (2)». Devuelve la ruta del archivo creado.",
            "inputSchema": object(json!({ "file": file, "slides": list(slide, MAX_SLIDES) }), &["file", "slides"]),
            "annotations": annotations,
        }),
    ]
}

// MARK: running

/// Writes the file `tool` describes inside `dir` and returns its path (absolute when `dir` is).
pub fn run(tool: &str, args: &Value, dir: &Path) -> Result<PathBuf, OfficeError> {
    let name = args["file"].as_str();
    match tool {
        DOCUMENT => {
            let blocks = parse_blocks(&args["blocks"])?;
            let title = clean(args["title"].as_str());
            write(&zip::archive(&docx::parts(title.as_deref(), &blocks))?, name, "docx", dir)
        }
        SPREADSHEET => write(&zip::archive(&xlsx::parts(&parse_sheets(&args["sheets"])?))?, name, "xlsx", dir),
        PRESENTATION => write(&zip::archive(&pptx::parts(&parse_slides(&args["slides"])?))?, name, "pptx", dir),
        _ => Err(OfficeError::new("Herramienta desconocida.")),
    }
}

// MARK: files

/// Names Windows keeps for devices: `CON.docx` would not be a file there.
const WINDOWS_RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// The stem of a file name that stays in the folder: no directories, letters, digits and ` _-()` only, at most 80
/// characters, never a Windows device name.
fn file_stem(raw: Option<&str>) -> Result<String, OfficeError> {
    let raw = raw.unwrap_or("").replace('\\', "/");
    let base = raw.split('/').rfind(|s| !s.is_empty()).unwrap_or("");
    // Whatever extension the agent wrote goes; the tool's own is added back.
    let stem = match base.rfind('.') {
        Some(dot) if base[dot + 1..].chars().count() <= 5 => &base[..dot],
        _ => base,
    };
    let cleaned: String =
        stem.chars().map(|c| if c.is_alphanumeric() || " _-()".contains(c) { c } else { '_' }).collect();
    let cleaned = cleaned.trim_matches(|c| c == ' ' || c == '.' || c == '_');
    if cleaned.is_empty() {
        return Err(OfficeError::new("Falta el nombre del archivo."));
    }
    let mut stem = xml::prefix(cleaned, MAX_FILE_NAME);
    if WINDOWS_RESERVED.iter().any(|r| r.eq_ignore_ascii_case(&stem)) {
        stem.push('_');
    }
    Ok(stem)
}

/// The file name the agent's `raw` becomes, with the tool's extension.
#[cfg(test)]
pub fn file_name(raw: Option<&str>, ext: &str) -> Result<String, OfficeError> {
    Ok(format!("{}.{ext}", file_stem(raw)?))
}

/// Creates `dir` (and its parents) when missing: 0700 on Unix, so other accounts cannot read what is written.
/// A folder that already exists keeps its permissions.
pub fn ensure_dir(dir: &Path) -> Result<(), OfficeError> {
    if dir.is_dir() {
        return Ok(());
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir).map_err(|_| OfficeError::new("No se pudo crear la carpeta de documentos."))
}

/// Writes `data` as a new file in `dir`. The file is created exclusively, so an existing file (or a link planted
/// under that name) is never written through: the next "name (n)" is tried instead.
fn write(data: &[u8], name: Option<&str>, ext: &str, dir: &Path) -> Result<PathBuf, OfficeError> {
    let stem = file_stem(name)?;
    ensure_dir(dir)?;
    for n in 1..MAX_VARIANTS {
        let file = if n == 1 { format!("{stem}.{ext}") } else { format!("{stem} ({n}).{ext}") };
        let path = dir.join(file);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut out) => {
                if out.write_all(data).and_then(|_| out.flush()).is_err() {
                    drop(out);
                    let _ = std::fs::remove_file(&path);
                    return Err(OfficeError::new("No se pudo guardar el archivo."));
                }
                return Ok(path);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(OfficeError::new("No se pudo guardar el archivo.")),
        }
    }
    Err(OfficeError::new("Ya hay demasiados archivos con ese nombre; usa otro."))
}

// MARK: parsing

fn clean(text: Option<&str>) -> Option<String> {
    let text = text?.trim();
    (!text.is_empty()).then(|| xml::prefix(text, MAX_TEXT))
}

fn cell_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.as_f64().map(xml::number_text).unwrap_or_default(),
        Value::Bool(b) => if *b { "Sí" } else { "No" }.to_string(),
        _ => String::new(),
    }
}

fn strings(value: &Value, limit: usize, what: &str) -> Result<Vec<String>, OfficeError> {
    let Some(items) = value.as_array() else { return Ok(Vec::new()) };
    if items.len() > limit {
        return Err(OfficeError(format!("Demasiados elementos en {what} (máximo {limit}).")));
    }
    Ok(items.iter().map(|v| xml::prefix(&cell_text(v), MAX_TEXT)).collect())
}

fn text_field(value: &Value) -> String {
    xml::prefix(value.as_str().unwrap_or(""), MAX_TEXT)
}

pub fn parse_blocks(value: &Value) -> Result<Vec<DocBlock>, OfficeError> {
    let raw = value.as_array().filter(|a| !a.is_empty());
    let Some(raw) = raw else { return Err(OfficeError::new("El documento no tiene contenido (blocks).")) };
    if raw.len() > MAX_BLOCKS {
        return Err(OfficeError(format!("Demasiados bloques (máximo {MAX_BLOCKS}).")));
    }
    raw.iter()
        .map(|block| {
            Ok(match block["type"].as_str().unwrap_or("") {
                "heading" => {
                    let level = block["level"].as_i64().or_else(|| block["level"].as_f64().map(|f| f as i64)).unwrap_or(1);
                    DocBlock::Heading { level, text: text_field(&block["text"]) }
                }
                "paragraph" => DocBlock::Paragraph(text_field(&block["text"])),
                "bullets" => DocBlock::Bullets(strings(&block["items"], MAX_ITEMS, "la lista")?),
                "numbered" => DocBlock::Numbered(strings(&block["items"], MAX_ITEMS, "la lista")?),
                "table" => {
                    let header = strings(&block["header"], MAX_COLUMNS, "el encabezado")?;
                    let rows = match block["rows"].as_array() {
                        None => Vec::new(),
                        Some(rows) if rows.len() > MAX_ROWS => {
                            return Err(OfficeError(format!("Demasiadas filas en la tabla (máximo {MAX_ROWS}).")));
                        }
                        Some(rows) => rows.iter().map(|r| strings(r, MAX_COLUMNS, "una fila")).collect::<Result<_, _>>()?,
                    };
                    DocBlock::Table { header, rows }
                }
                "pagebreak" => DocBlock::PageBreak,
                _ => {
                    return Err(OfficeError::new(
                        "Tipo de bloque desconocido. Usa heading, paragraph, bullets, numbered, table o pagebreak.",
                    ));
                }
            })
        })
        .collect()
}

fn sheet_cell(item: &Value) -> SheetCell {
    match item {
        Value::String(s) if s.starts_with('=') && s.len() > 1 => SheetCell::Formula(xml::prefix(&s[1..], MAX_TEXT)),
        Value::String(s) => SheetCell::Text(xml::prefix(s, MAX_TEXT)),
        Value::Number(n) => n.as_f64().filter(|f| f.is_finite()).map(SheetCell::Number).unwrap_or(SheetCell::Empty),
        Value::Bool(b) => SheetCell::Bool(*b),
        _ => SheetCell::Empty,
    }
}

pub fn parse_sheets(value: &Value) -> Result<Vec<SheetData>, OfficeError> {
    let raw = value.as_array().filter(|a| !a.is_empty());
    let Some(raw) = raw else { return Err(OfficeError::new("El libro no tiene hojas (sheets).")) };
    if raw.len() > MAX_SHEETS {
        return Err(OfficeError(format!("Demasiadas hojas (máximo {MAX_SHEETS}).")));
    }
    let mut sheets = raw
        .iter()
        .enumerate()
        .map(|(index, sheet)| {
            let rows = sheet["rows"].as_array().filter(|r| r.len() <= MAX_ROWS);
            let Some(rows) = rows else {
                return Err(OfficeError(format!("Cada hoja necesita `rows` (máximo {MAX_ROWS} filas).")));
            };
            let cells = rows
                .iter()
                .map(|row| match row.as_array() {
                    Some(items) if items.len() <= MAX_COLUMNS => Ok(items.iter().map(sheet_cell).collect()),
                    _ => Err(OfficeError(format!("Cada fila es una lista de celdas (máximo {MAX_COLUMNS})."))),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(SheetData {
                name: xlsx::sheet_name(sheet["name"].as_str().unwrap_or(""), index),
                rows: cells,
                header: sheet["header"].as_bool().unwrap_or(true),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    xlsx::unique_sheet_names(&mut sheets);
    Ok(sheets)
}

pub fn parse_slides(value: &Value) -> Result<Vec<SlideData>, OfficeError> {
    let raw = value.as_array().filter(|a| !a.is_empty());
    let Some(raw) = raw else { return Err(OfficeError::new("La presentación no tiene diapositivas (slides).")) };
    if raw.len() > MAX_SLIDES {
        return Err(OfficeError(format!("Demasiadas diapositivas (máximo {MAX_SLIDES}).")));
    }
    raw.iter()
        .map(|slide| {
            Ok(SlideData {
                title: xml::prefix(slide["title"].as_str().unwrap_or(""), MAX_SLIDE_TITLE),
                subtitle: slide["subtitle"].as_str().map(|s| xml::prefix(s, MAX_SLIDE_TITLE)),
                bullets: strings(&slide["bullets"], MAX_BULLETS, "las viñetas")?,
            })
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A fresh, empty folder under the system temp dir, removed when dropped.
    pub struct TempDir(pub PathBuf);

    impl TempDir {
        pub fn new(tag: &str) -> Self {
            static N: AtomicUsize = AtomicUsize::new(0);
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
            let path = std::env::temp_dir().join(format!(
                "buddy-hook-{tag}-{}-{nanos}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::TempDir;
    use super::*;

    fn read_parts(path: &Path) -> Vec<(String, Vec<u8>)> {
        zip::read(&std::fs::read(path).unwrap()).expect("a valid zip")
    }

    fn names(parts: &[(String, Vec<u8>)]) -> Vec<&str> {
        parts.iter().map(|(n, _)| n.as_str()).collect()
    }

    #[test]
    fn file_names_stay_in_the_folder() {
        assert_eq!(file_name(Some("Informe 2026"), "docx").unwrap(), "Informe 2026.docx");
        assert_eq!(file_name(Some("informe.xlsx"), "docx").unwrap(), "informe.docx", "the extension is the tool's");
        assert_eq!(file_name(Some("../../etc/passwd"), "docx").unwrap(), "passwd.docx");
        assert_eq!(file_name(Some("/etc/passwd"), "docx").unwrap(), "passwd.docx");
        assert_eq!(file_name(Some("C:\\Windows\\win.ini"), "docx").unwrap(), "win.docx");
        assert_eq!(file_name(Some("a\\b\\c:d?.pptx"), "pptx").unwrap(), "c_d.pptx", "unsafe characters become _");
        assert_eq!(file_name(Some("Año ñandú"), "xlsx").unwrap(), "Año ñandú.xlsx");
        assert_eq!(file_name(Some("informe.final.v2"), "docx").unwrap(), "informe_final.docx");
        assert_eq!(file_name(Some("con"), "docx").unwrap(), "con_.docx", "never a Windows device name");
        assert_eq!(file_name(Some(&"x".repeat(200)), "docx").unwrap().chars().count(), 85);
        for bad in [None, Some(""), Some("..."), Some(".."), Some("../"), Some("/"), Some("___"), Some("   ")] {
            assert!(file_name(bad, "docx").is_err(), "{bad:?}");
        }
    }

    #[test]
    fn files_never_overwrite_and_never_leave_the_folder() {
        let tmp = TempDir::new("names");
        let dir = tmp.0.join("documentos");
        let args = json!({ "file": "../x.docx", "blocks": [{ "type": "paragraph", "text": "hola" }] });
        let first = run(DOCUMENT, &args, &dir).unwrap();
        let second = run(DOCUMENT, &args, &dir).unwrap();
        assert_eq!(first, dir.join("x.docx"));
        assert_eq!(second, dir.join("x (2).docx"));
        assert!(!tmp.0.join("x.docx").exists(), "nothing escapes the folder");

        let abs = json!({ "file": tmp.0.join("fuera.docx").to_string_lossy(), "blocks": [{ "type": "pagebreak" }] });
        assert_eq!(run(DOCUMENT, &abs, &dir).unwrap(), dir.join("fuera.docx"));
        assert!(!tmp.0.join("fuera.docx").exists());

        let empty = json!({ "file": "", "blocks": [{ "type": "pagebreak" }] });
        assert_eq!(run(DOCUMENT, &empty, &dir).unwrap_err(), OfficeError::new("Falta el nombre del archivo."));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "the folder is private");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_planted_link_is_not_written_through() {
        let tmp = TempDir::new("link");
        let dir = tmp.0.join("docs");
        ensure_dir(&dir).unwrap();
        let target = tmp.0.join("victim.txt");
        std::fs::write(&target, "intacto").unwrap();
        std::os::unix::fs::symlink(&target, dir.join("x.docx")).unwrap();
        let path = run(DOCUMENT, &json!({ "file": "x", "blocks": [{ "type": "pagebreak" }] }), &dir).unwrap();
        assert_eq!(path, dir.join("x (2).docx"));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "intacto");
    }

    #[test]
    fn limits_and_shapes_are_enforced() {
        assert!(parse_blocks(&json!([])).is_err());
        assert!(parse_blocks(&json!("hola")).is_err());
        assert!(parse_blocks(&json!([{ "type": "video" }])).is_err());
        assert!(parse_blocks(&json!([{ "text": "sin tipo" }])).is_err());
        let many = vec![json!({ "type": "pagebreak" }); MAX_BLOCKS + 1];
        assert!(parse_blocks(&Value::Array(many)).is_err());
        assert!(parse_blocks(&json!([{ "type": "bullets", "items": vec!["x"; MAX_ITEMS + 1] }])).is_err());
        assert!(parse_blocks(&json!([{ "type": "table", "header": vec!["h"; MAX_COLUMNS + 1] }])).is_err());
        assert!(parse_blocks(&json!([{ "type": "table", "rows": vec![vec!["c"]; MAX_ROWS + 1] }])).is_err());
        assert_eq!(parse_blocks(&json!([{ "type": "paragraph", "text": "x".repeat(MAX_TEXT + 10) }])).unwrap(), vec![
            DocBlock::Paragraph("x".repeat(MAX_TEXT))
        ]);
        assert_eq!(
            parse_blocks(&json!([{ "type": "bullets", "items": ["a", 2, 2.5, true, null] }])).unwrap(),
            vec![DocBlock::Bullets(vec!["a".into(), "2".into(), "2.5".into(), "Sí".into(), String::new()])]
        );
        assert_eq!(parse_blocks(&json!([{ "type": "heading", "level": 2.0, "text": "T" }])).unwrap(), vec![
            DocBlock::Heading { level: 2, text: "T".into() }
        ]);

        assert!(parse_sheets(&json!([])).is_err());
        assert!(parse_sheets(&json!([{ "name": "x" }])).is_err(), "rows are required");
        assert!(parse_sheets(&Value::Array(vec![json!({ "rows": [] }); MAX_SHEETS + 1])).is_err());
        assert!(parse_sheets(&json!([{ "rows": vec![vec![1]; MAX_ROWS + 1] }])).is_err());
        assert!(parse_sheets(&json!([{ "rows": [vec![1; MAX_COLUMNS + 1]] }])).is_err());
        assert!(parse_sheets(&json!([{ "rows": ["no es una fila"] }])).is_err());

        assert!(parse_slides(&json!([])).is_err());
        assert!(parse_slides(&Value::Array(vec![json!({ "title": "t" }); MAX_SLIDES + 1])).is_err());
        assert!(parse_slides(&json!([{ "title": "t", "bullets": vec!["b"; MAX_BULLETS + 1] }])).is_err());
        let slides = parse_slides(&json!([{ "title": "t".repeat(400), "subtitle": "s" }])).unwrap();
        assert_eq!(slides[0].title.chars().count(), 300);
        assert_eq!(slides[0].subtitle.as_deref(), Some("s"));
    }

    #[test]
    fn sheets_keep_formulas_types_and_safe_names() {
        let sheets = parse_sheets(&json!([
            { "name": "Ventas: Q1/2026 con un nombre larguísimo", "rows": [["N", "V"], ["a", 1.5], ["b", "=SUM(B2:B2)"], [true, null], ["="]] },
            { "rows": [["x"]], "header": false },
            { "name": "hoja2", "rows": [] }
        ]))
        .unwrap();
        assert_eq!(sheets[0].name.chars().count(), 31);
        assert_eq!(sheets[1].name, "Hoja2");
        assert_eq!(sheets[2].name, "hoja2 (2)", "names are unique, ignoring case");
        assert!(!sheets[1].header);
        assert_eq!(sheets[0].rows[2][1], SheetCell::Formula("SUM(B2:B2)".into()));
        assert_eq!(sheets[0].rows[3], vec![SheetCell::Bool(true), SheetCell::Empty]);
        assert_eq!(sheets[0].rows[4], vec![SheetCell::Text("=".into())], "a lone = is text");
    }

    #[test]
    fn each_tool_writes_a_valid_zip_with_its_parts() {
        let tmp = TempDir::new("parts");
        let doc = run(
            DOCUMENT,
            &json!({ "file": "d", "title": "Título <1>", "blocks": [
                { "type": "heading", "level": 1, "text": "Uno & dos" },
                { "type": "paragraph", "text": "a **b** \"c\"" },
                { "type": "table", "header": ["A", "B"], "rows": [["1", "2"]] }
            ] }),
            &tmp.0,
        )
        .unwrap();
        let parts = read_parts(&doc);
        assert_eq!(names(&parts)[..3], ["[Content_Types].xml", "_rels/.rels", "word/document.xml"]);
        let document = String::from_utf8(parts[2].1.clone()).unwrap();
        assert!(document.contains("Título &lt;1&gt;") && document.contains("Uno &amp; dos") && document.contains("&quot;c&quot;"));

        let book = run(SPREADSHEET, &json!({ "file": "b", "sheets": [{ "name": "S", "rows": [["a"], [1]] }, { "rows": [[2]] }] }), &tmp.0).unwrap();
        let parts = read_parts(&book);
        for name in ["[Content_Types].xml", "xl/workbook.xml", "xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"] {
            assert!(names(&parts).contains(&name), "{name}");
        }

        let deck = run(PRESENTATION, &json!({ "file": "p", "slides": [{ "title": "Plan" }, { "title": "A", "bullets": ["x"] }] }), &tmp.0).unwrap();
        let parts = read_parts(&deck);
        for name in ["[Content_Types].xml", "ppt/presentation.xml", "ppt/slides/slide1.xml", "ppt/slides/slide2.xml"] {
            assert!(names(&parts).contains(&name), "{name}");
        }

        assert_eq!(run("borrar_todo", &json!({}), &tmp.0).unwrap_err(), OfficeError::new("Herramienta desconocida."));
    }

    /// A second opinion from Python's own zipfile and XML parser, when this machine has Python.
    #[test]
    fn python_reads_every_file_back() {
        let python_ok = std::process::Command::new("python3")
            .args(["-c", "import zipfile, xml.dom.minidom"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !python_ok {
            eprintln!("python3 is not available; skipping the zipfile check");
            return;
        }
        let tmp = TempDir::new("python");
        let files = [
            run(
                DOCUMENT,
                &json!({ "file": "doc", "title": "Informe", "blocks": [
                    { "type": "heading", "level": 2, "text": "A & B <c> \"d\" 'e'" },
                    { "type": "paragraph", "text": "**negrita** *cursiva* `código` \u{1}" },
                    { "type": "bullets", "items": ["uno", "dos"] },
                    { "type": "numbered", "items": ["tres"] },
                    { "type": "table", "header": ["x"], "rows": [["1", "2"]] },
                    { "type": "pagebreak" }
                ] }),
                &tmp.0,
            )
            .unwrap(),
            run(
                SPREADSHEET,
                &json!({ "file": "libro", "sheets": [{ "name": "A&B", "rows": [["n", "v"], ["<x>", 3], ["s", "=SUM(B2:B2)"], [false, null]] }] }),
                &tmp.0,
            )
            .unwrap(),
            run(
                PRESENTATION,
                &json!({ "file": "deck", "slides": [{ "title": "T & T", "subtitle": "<sub>" }, { "title": "B", "bullets": ["**a**", "b > c"] }] }),
                &tmp.0,
            )
            .unwrap(),
        ];
        let script = "import sys, zipfile, xml.dom.minidom\n\
                      for p in sys.argv[1:]:\n\
                      \x20   z = zipfile.ZipFile(p)\n\
                      \x20   assert z.testzip() is None, p\n\
                      \x20   for n in z.namelist():\n\
                      \x20       xml.dom.minidom.parseString(z.read(n))\n\
                      print('ok')\n";
        let out = std::process::Command::new("python3").arg("-c").arg(script).args(&files).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ok");
    }
}
