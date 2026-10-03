// Ported from MIKA (MIT, revision d050bc5): apps/macos/Sources/Providers/OfficeFiles.swift (XlsxWriter)
//! An Excel workbook (.xlsx): sheets of text, numbers, booleans, dates and formulas, the first row optionally a
//! bold header on Buddy's indigo, frozen and filtered, columns as wide as their content (or as asked) and number
//! formats per column (currency, percent, date…). Formulas are recalculated when the book opens.

use super::xml::{self, Part};

/// One cell.
#[derive(Clone, Debug, PartialEq)]
pub enum SheetCell {
    Text(String),
    Number(f64),
    Bool(bool),
    /// Without the leading `=`.
    Formula(String),
    Empty,
}

/// How one column looks: its width in characters and its number format code (Excel's own syntax).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColumnSpec {
    pub width: Option<f64>,
    pub format: Option<String>,
}

/// One sheet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SheetData {
    pub name: String,
    pub rows: Vec<Vec<SheetCell>>,
    /// The first row is a header: bold on a band, frozen.
    pub header: bool,
    /// A filter on the header row (only with a header and some data under it).
    pub autofilter: bool,
    pub columns: Vec<ColumnSpec>,
}

/// The number format code for one of the names the tool accepts, or the agent's own code (Excel syntax).
pub fn format_code(name: &str) -> Option<String> {
    let code = match name.trim().to_lowercase().as_str() {
        "" | "general" => return None,
        "text" | "texto" => "@",
        "integer" | "entero" => "#,##0",
        "number" | "decimal" | "número" | "numero" => "#,##0.00",
        "currency" | "moneda" | "pen" | "soles" => "\"S/\" #,##0.00",
        "usd" | "dólares" | "dolares" => "\"$\"#,##0.00",
        "eur" | "euros" => "#,##0.00 \"€\"",
        "percent" | "porcentaje" => "0.0%",
        "date" | "fecha" => "dd/mm/yyyy",
        "datetime" | "fecha_hora" => "dd/mm/yyyy hh:mm",
        "time" | "hora" => "hh:mm",
        _ => return Some(xml::prefix(name.trim(), 64)),
    };
    Some(code.to_string())
}

/// Is this format code a date or time? Then ISO texts in the column become real dates.
fn is_date_format(code: &str) -> bool {
    let mut quoted = false;
    let mut bracket = false;
    for c in code.chars() {
        match c {
            '"' => quoted = !quoted,
            '[' if !quoted => bracket = true,
            ']' if !quoted => bracket = false,
            'd' | 'm' | 'y' | 'h' | 's' | 'D' | 'M' | 'Y' | 'H' | 'S' if !quoted && !bracket => return true,
            _ => {}
        }
    }
    false
}

/// `2026-10-02`, `2026-10-02T14:30`, `2026-10-02 14:30:15` or `02/10/2026` as Excel's serial date (days since
/// 1899-12-30, the time as a fraction).
pub fn date_serial(text: &str) -> Option<f64> {
    let t = text.trim();
    let (date, time) = match t.split_once(['T', ' ']) {
        Some((d, rest)) => (d, Some(rest.trim_end_matches('Z'))),
        None => (t, None),
    };
    let numbers = |s: &str, sep: char| -> Option<Vec<u32>> { s.split(sep).map(|p| p.parse::<u32>().ok()).collect() };
    let (y, m, d) = if let Some(p) = numbers(date, '-').filter(|p| p.len() == 3 && date.len() == 10) {
        (p[0] as i64, p[1], p[2])
    } else {
        let p = numbers(date, '/').filter(|p| p.len() == 3 && p[2] >= 1000)?;
        (p[2] as i64, p[1], p[0])
    };
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || !(1900..=9999).contains(&y) {
        return None;
    }
    let days = xml::days_from_civil(y, m, d);
    if xml::civil_from_days(days) != (y, m, d) {
        return None; // 31 de febrero
    }
    let mut serial = (days - xml::days_from_civil(1899, 12, 30)) as f64;
    if let Some(time) = time.filter(|t| !t.is_empty()) {
        let parts: Vec<u32> = time.split(':').map(|p| p.split('.').next().unwrap_or("").parse().ok()).collect::<Option<_>>()?;
        let (h, min, sec) = (parts[0], *parts.get(1).unwrap_or(&0), *parts.get(2).unwrap_or(&0));
        if parts.len() > 3 || h > 23 || min > 59 || sec > 59 {
            return None;
        }
        serial += (h * 3600 + min * 60 + sec) as f64 / 86_400.0;
    }
    Some(serial)
}

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
/// Excel's limit for a sheet name, in UTF-16 units.
const MAX_SHEET_NAME: usize = 31;

/// The first `max` UTF-16 units of `text`, never splitting a character.
fn prefix_utf16(text: &str, max: usize) -> String {
    let mut used = 0;
    text.chars()
        .take_while(|c| {
            used += c.len_utf16();
            used <= max
        })
        .collect()
}

/// A sheet name Excel accepts: at most 31 characters, none of `[]:*?/\`, no quote or space at either end, not empty.
pub fn sheet_name(raw: &str, index: usize) -> String {
    let cleaned: String = raw.chars().map(|c| if "[]:*?/\\".contains(c) { '_' } else { c }).collect();
    let cleaned = cleaned.trim_matches(|c: char| c == '\'' || c.is_whitespace());
    let name = if cleaned.is_empty() { format!("Hoja{}", index + 1) } else { cleaned.to_string() };
    prefix_utf16(&name, MAX_SHEET_NAME)
}

/// Excel refuses a workbook whose sheet names repeat (ignoring case): later ones get " (2)", " (3)"…
pub fn unique_sheet_names(sheets: &mut [SheetData]) {
    let mut taken: Vec<String> = Vec::new();
    for sheet in sheets.iter_mut() {
        let mut name = sheet.name.clone();
        let mut n = 2;
        while taken.contains(&name.to_lowercase()) {
            let suffix = format!(" ({n})");
            name = prefix_utf16(&sheet.name, MAX_SHEET_NAME - suffix.len()) + &suffix;
            n += 1;
        }
        taken.push(name.to_lowercase());
        sheet.name = name;
    }
}

/// The cell styles of a workbook: 0 plain, 1 header, then one per number format in use.
struct Styles {
    formats: Vec<String>,
}

impl Styles {
    fn new(sheets: &[SheetData]) -> Self {
        let mut formats: Vec<String> = Vec::new();
        for column in sheets.iter().flat_map(|s| &s.columns) {
            if let Some(f) = &column.format
                && !formats.contains(f)
            {
                formats.push(f.clone());
            }
        }
        Styles { formats }
    }

    /// The `s` index for a body cell with this format.
    fn index(&self, format: Option<&String>) -> usize {
        format.and_then(|f| self.formats.iter().position(|x| x == f)).map(|i| i + 2).unwrap_or(0)
    }

    fn xml(&self) -> String {
        let mut num_fmts = String::new();
        let mut xfs = String::from(
            r#"<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="2" borderId="1" xfId="0" applyFont="1" applyFill="1" applyBorder="1" applyAlignment="1"><alignment vertical="center" wrapText="1"/></xf>"#,
        );
        for (i, code) in self.formats.iter().enumerate() {
            let id = 164 + i;
            num_fmts.push_str(&format!("<numFmt numFmtId=\"{id}\" formatCode=\"{}\"/>", xml::escape(code)));
            xfs.push_str(&format!("<xf numFmtId=\"{id}\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>"));
        }
        let num_fmts = if self.formats.is_empty() {
            String::new()
        } else {
            format!("<numFmts count=\"{}\">{num_fmts}</numFmts>", self.formats.len())
        };
        [
            format!("<styleSheet xmlns=\"{MAIN}\">{num_fmts}"),
            r#"<fonts count="2"><font><sz val="11"/><name val="Calibri"/><family val="2"/></font><font><b/><sz val="11"/><color rgb="FFFFFFFF"/><name val="Calibri"/><family val="2"/></font></fonts>"#.into(),
            r#"<fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FF2E3BA8"/><bgColor indexed="64"/></patternFill></fill></fills>"#.into(),
            r#"<borders count="2"><border><left/><right/><top/><bottom/><diagonal/></border><border><left/><right/><top/><bottom style="medium"><color rgb="FF1F9D57"/></bottom><diagonal/></border></borders>"#.into(),
            r#"<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>"#.into(),
            format!("<cellXfs count=\"{}\">{xfs}</cellXfs>", self.formats.len() + 2),
            r#"<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles><dxfs count="0"/><tableStyles count="0"/></styleSheet>"#.into(),
        ]
        .concat()
    }
}

/// How many characters a cell needs, roughly, once formatted.
fn cell_width(cell: &SheetCell, format: Option<&str>) -> usize {
    match cell {
        SheetCell::Text(t) => t.lines().map(|l| l.chars().count()).max().unwrap_or(0),
        SheetCell::Number(n) => match format {
            Some(f) if is_date_format(f) => f.chars().count().max(10),
            Some(f) => {
                let digits = xml::number_text(n.trunc()).len();
                digits + digits / 3 + f.chars().filter(|c| !"#0,.".contains(*c)).count().min(6) + 3
            }
            None => xml::number_text(*n).len(),
        },
        SheetCell::Bool(_) => 9,
        SheetCell::Formula(_) => 12,
        SheetCell::Empty => 0,
    }
}

fn sheet_xml(sheet: &SheetData, styles: &Styles) -> String {
    let columns = sheet.rows.iter().map(Vec::len).max().unwrap_or(1).max(1).max(sheet.columns.len());
    let spec = |c: usize| sheet.columns.get(c);
    let format = |c: usize| spec(c).and_then(|s| s.format.as_ref());
    let mut widths = vec![8.0f64; columns];
    for (r, row) in sheet.rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate().take(columns) {
            let mut w = cell_width(cell, format(c).map(String::as_str)) as f64 + 2.0;
            if sheet.header && r == 0 {
                w = w * 1.1 + 3.0; // bold, and room for the filter button
            }
            widths[c] = widths[c].max(w).min(60.0);
        }
    }
    for (c, width) in widths.iter_mut().enumerate() {
        if let Some(w) = spec(c).and_then(|s| s.width) {
            *width = w.clamp(2.0, 120.0);
        }
    }
    let frozen = sheet.header && sheet.rows.len() > 1;
    let mut out = format!("<worksheet xmlns=\"{MAIN}\" xmlns:r=\"{REL}\"><sheetViews><sheetView workbookViewId=\"0\">");
    if frozen {
        out.push_str(r#"<pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/><selection pane="bottomLeft" activeCell="A2" sqref="A2"/>"#);
    }
    out.push_str("</sheetView></sheetViews><sheetFormatPr defaultRowHeight=\"15\"/><cols>");
    for (c, width) in widths.iter().enumerate() {
        let style = styles.index(format(c));
        let style = if style > 0 { format!(" style=\"{style}\"") } else { String::new() };
        out.push_str(&format!("<col min=\"{0}\" max=\"{0}\" width=\"{1}\"{style} customWidth=\"1\"/>", c + 1, xml::number_text((width * 100.0).round() / 100.0)));
    }
    out.push_str("</cols><sheetData>");
    for (r, row) in sheet.rows.iter().enumerate() {
        let head = sheet.header && r == 0;
        out.push_str(&if head { format!("<row r=\"{}\" ht=\"20\" customHeight=\"1\">", r + 1) } else { format!("<row r=\"{}\">", r + 1) });
        for (c, cell) in row.iter().enumerate() {
            let at = format!("{}{}", xml::column(c), r + 1);
            let index = if head { 1 } else { styles.index(format(c)) };
            let style = if index > 0 { format!(" s=\"{index}\"") } else { String::new() };
            // A date column turns ISO dates into real dates; a percent column turns "25%" into 0.25.
            let converted = match (cell, format(c)) {
                (SheetCell::Text(t), Some(f)) if !head && is_date_format(f) => date_serial(t).map(SheetCell::Number),
                (SheetCell::Text(t), Some(f)) if !head && f.ends_with('%') => {
                    t.trim().strip_suffix('%').and_then(|n| n.trim().replace(',', ".").parse::<f64>().ok()).map(|n| SheetCell::Number(n / 100.0))
                }
                _ => None,
            };
            match converted.as_ref().unwrap_or(cell) {
                SheetCell::Empty => {
                    if head {
                        out.push_str(&format!("<c r=\"{at}\"{style}/>"));
                    }
                }
                SheetCell::Text(t) => out.push_str(&format!(
                    "<c r=\"{at}\"{style} t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",
                    xml::escape(t)
                )),
                SheetCell::Number(n) => out.push_str(&format!("<c r=\"{at}\"{style}><v>{}</v></c>", number_value(*n))),
                SheetCell::Bool(b) => out.push_str(&format!("<c r=\"{at}\"{style} t=\"b\"><v>{}</v></c>", u8::from(*b))),
                SheetCell::Formula(f) => out.push_str(&format!("<c r=\"{at}\"{style}><f>{}</f></c>", xml::escape(f))),
            }
        }
        out.push_str("</row>");
    }
    out.push_str("</sheetData>");
    if frozen && sheet.autofilter {
        out.push_str(&format!("<autoFilter ref=\"A1:{}{}\"/>", xml::column(columns - 1), sheet.rows.len()));
    }
    out.push_str(r#"<pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/>"#);
    out + "</worksheet>"
}

/// A number as a cell value: full precision, never an exponent Excel would misread.
fn number_value(n: f64) -> String {
    if n == n.trunc() && n.abs() < 1e15 { (n as i64).to_string() } else { format!("{n}") }
}

/// The parts of an .xlsx.
pub fn parts(sheets: &[SheetData]) -> Vec<Part> {
    let count = sheets.len();
    let mut types = String::from(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#,
    );
    types.push_str(&format!(
        "<Override PartName=\"/docProps/core.xml\" ContentType=\"{}\"/><Override PartName=\"/docProps/app.xml\" ContentType=\"{}\"/>",
        xml::CORE_TYPE,
        xml::APP_TYPE
    ));
    for i in 1..=count {
        types.push_str(&format!(
            "<Override PartName=\"/xl/worksheets/sheet{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>"
        ));
    }
    types.push_str("</Types>");
    let rels = format!(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>{}</Relationships>",
        xml::PROPS_RELS
    );
    let mut sheet_list = String::new();
    let mut workbook_rels = String::from(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for (i, sheet) in sheets.iter().enumerate() {
        let n = i + 1;
        sheet_list.push_str(&format!("<sheet name=\"{}\" sheetId=\"{n}\" r:id=\"rId{n}\"/>", xml::escape(&sheet.name)));
        workbook_rels.push_str(&format!(
            "<Relationship Id=\"rId{n}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{n}.xml\"/>"
        ));
    }
    workbook_rels.push_str(&format!(
        "<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>",
        count + 1
    ));
    let workbook = format!(
        "<workbook xmlns=\"{MAIN}\" xmlns:r=\"{REL}\"><bookViews><workbookView/></bookViews><sheets>{sheet_list}</sheets><calcPr calcId=\"191029\" fullCalcOnLoad=\"1\"/></workbook>"
    );
    let styles = Styles::new(sheets);
    let mut out = vec![
        xml::part("[Content_Types].xml", &types),
        xml::part("_rels/.rels", &rels),
        xml::part("xl/workbook.xml", &workbook),
        xml::part("xl/_rels/workbook.xml.rels", &workbook_rels),
        xml::part("xl/styles.xml", &styles.xml()),
    ];
    for (i, sheet) in sheets.iter().enumerate() {
        out.push(xml::part(&format!("xl/worksheets/sheet{}.xml", i + 1), &sheet_xml(sheet, &styles)));
    }
    out.extend(xml::properties(None, None, None));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_names_are_safe_and_short() {
        let name = sheet_name("Ventas: Q1/2026 con un nombre larguísimo", 0);
        assert_eq!(name.chars().count(), 31);
        assert!(!name.contains(':') && !name.contains('/'));
        assert_eq!(sheet_name("  ' ", 1), "Hoja2");
        assert_eq!(sheet_name("'Datos'", 0), "Datos");
        assert_eq!(sheet_name(&"😀".repeat(20), 0).chars().count(), 15, "31 UTF-16 units, never half an emoji");
    }

    #[test]
    fn repeated_sheet_names_are_told_apart() {
        let sheet = |name: &str| SheetData { name: name.into(), header: true, ..SheetData::default() };
        let mut sheets = vec![sheet("Datos"), sheet("datos"), sheet("Datos"), sheet(&"x".repeat(31)), sheet(&"x".repeat(31))];
        unique_sheet_names(&mut sheets);
        let names: Vec<&str> = sheets.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(&names[..3], ["Datos", "datos (2)", "Datos (3)"]);
        assert_eq!(names[4].chars().count(), 31);
        assert!(names[4].ends_with(" (2)"));
    }

    #[test]
    fn the_workbook_keeps_formulas_types_and_escapes() {
        let sheets = vec![
            SheetData {
                name: "A&B".into(),
                rows: vec![
                    vec![SheetCell::Text("N".into()), SheetCell::Text("V".into())],
                    vec![SheetCell::Text("a < b".into()), SheetCell::Number(1.5)],
                    vec![SheetCell::Text("b".into()), SheetCell::Formula("IF(B2>1,\"sí\",\"no\")".into())],
                    vec![SheetCell::Bool(true), SheetCell::Empty],
                ],
                header: true,
                autofilter: true,
                columns: vec![],
            },
            SheetData { name: "Hoja2".into(), rows: vec![vec![SheetCell::Text("x".into())]], ..SheetData::default() },
        ];
        let parts = parts(&sheets);
        let names: Vec<&str> = parts.iter().map(|(p, _)| p.as_str()).collect();
        for name in ["[Content_Types].xml", "xl/workbook.xml", "xl/styles.xml", "xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"] {
            assert!(names.contains(&name), "{name}");
        }
        let text: String = parts.iter().map(|(_, d)| String::from_utf8_lossy(d).into_owned()).collect();
        for needle in [
            "<sheet name=\"A&amp;B\"",
            "<f>IF(B2&gt;1,&quot;sí&quot;,&quot;no&quot;)</f>",
            "t=\"inlineStr\"><is><t xml:space=\"preserve\">a &lt; b</t>",
            "<v>1.5</v>",
            "t=\"b\"><v>1</v>",
            "<autoFilter ref=\"A1:B4\"/>",
            "state=\"frozen\"",
        ] {
            assert!(text.contains(needle), "{needle}");
        }
        let second = String::from_utf8(parts.iter().find(|(p, _)| p == "xl/worksheets/sheet2.xml").unwrap().1.clone()).unwrap();
        assert!(!second.contains("frozen") && !second.contains("autoFilter") && !second.contains("s=\"1\""));
        assert!(text.contains("fullCalcOnLoad=\"1\""), "formulas are computed when the book opens");
    }

    #[test]
    fn formats_dates_and_widths() {
        assert_eq!(format_code("currency").as_deref(), Some("\"S/\" #,##0.00"));
        assert_eq!(format_code("Percent").as_deref(), Some("0.0%"));
        assert_eq!(format_code("0.000").as_deref(), Some("0.000"), "Excel's own codes pass through");
        assert_eq!(format_code(""), None);
        assert!(is_date_format("dd/mm/yyyy") && is_date_format("hh:mm") && !is_date_format("\"S/\" #,##0.00") && !is_date_format("0.0%"));
        assert_eq!(date_serial("2026-10-02"), Some(46_297.0));
        assert_eq!(date_serial("02/10/2026"), Some(46_297.0));
        assert_eq!(date_serial("2026-10-02T12:00"), Some(46_297.5));
        assert_eq!(date_serial("1900-03-01"), Some(61.0));
        for bad in ["2026-02-31", "hoy", "2026-13-01", "2026-10-02T25:00", "12/2026"] {
            assert_eq!(date_serial(bad), None, "{bad}");
        }

        let sheet = SheetData {
            name: "Ventas".into(),
            rows: vec![
                vec![SheetCell::Text("Fecha".into()), SheetCell::Text("Monto".into()), SheetCell::Text("Margen".into())],
                vec![SheetCell::Text("2026-10-02".into()), SheetCell::Number(1234.5), SheetCell::Text("25%".into())],
                vec![SheetCell::Text("no es fecha".into()), SheetCell::Number(10.0), SheetCell::Number(0.3)],
            ],
            header: true,
            autofilter: false,
            columns: vec![
                ColumnSpec { width: None, format: format_code("date") },
                ColumnSpec { width: Some(20.0), format: format_code("currency") },
                ColumnSpec { width: None, format: format_code("percent") },
            ],
        };
        let parts = parts(std::slice::from_ref(&sheet));
        let text = |path: &str| String::from_utf8(parts.iter().find(|(p, _)| p == path).unwrap().1.clone()).unwrap();
        let styles = text("xl/styles.xml");
        assert!(styles.contains("<numFmt numFmtId=\"164\" formatCode=\"dd/mm/yyyy\"/>"));
        assert!(styles.contains("formatCode=\"&quot;S/&quot; #,##0.00\""));
        assert!(styles.contains("<cellXfs count=\"5\">"));
        let xml = text("xl/worksheets/sheet1.xml");
        assert!(xml.contains("<c r=\"A2\" s=\"2\"><v>46297</v></c>"), "{xml}");
        assert!(xml.contains("<c r=\"A3\" s=\"2\" t=\"inlineStr\">"), "a text that is not a date stays text");
        assert!(xml.contains("<c r=\"B2\" s=\"3\"><v>1234.5</v></c>"));
        assert!(xml.contains("<c r=\"C2\" s=\"4\"><v>0.25</v></c>"));
        assert!(xml.contains("<c r=\"A1\" s=\"1\""), "the header keeps its own style");
        assert!(xml.contains("<col min=\"2\" max=\"2\" width=\"20\" style=\"3\""));
        assert!(xml.contains("state=\"frozen\"") && !xml.contains("autoFilter"), "frozen, no filter when not asked");
    }
}
