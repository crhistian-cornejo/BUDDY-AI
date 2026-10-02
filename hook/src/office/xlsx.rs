// Ported from MIKA (MIT, revision d050bc5): apps/macos/Sources/Providers/OfficeFiles.swift (XlsxWriter)
//! An Excel workbook (.xlsx): sheets of text, numbers, booleans and formulas, the first row optionally a bold,
//! frozen, filtered header, and columns as wide as their content.

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

/// One sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetData {
    pub name: String,
    pub rows: Vec<Vec<SheetCell>>,
    /// The first row is a header: bold on a band, frozen, with a filter.
    pub header: bool,
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

fn sheet_xml(sheet: &SheetData) -> String {
    let columns = sheet.rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let mut widths = vec![8usize; columns];
    for row in &sheet.rows {
        for (c, cell) in row.iter().enumerate().take(columns) {
            let length = match cell {
                SheetCell::Text(t) => t.chars().count() + 2,
                SheetCell::Number(n) => xml::number_text(*n).len() + 2,
                SheetCell::Bool(_) => 7,
                SheetCell::Formula(_) => 12,
                SheetCell::Empty => 0,
            };
            widths[c] = widths[c].max(length).min(60);
        }
    }
    let frozen = sheet.header && sheet.rows.len() > 1;
    let mut out = format!("<worksheet xmlns=\"{MAIN}\"><sheetViews><sheetView workbookViewId=\"0\">");
    if frozen {
        out.push_str(r#"<pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/>"#);
    }
    out.push_str("</sheetView></sheetViews><sheetFormatPr defaultRowHeight=\"15\"/><cols>");
    for (c, width) in widths.iter().enumerate() {
        out.push_str(&format!("<col min=\"{0}\" max=\"{0}\" width=\"{width}\" customWidth=\"1\"/>", c + 1));
    }
    out.push_str("</cols><sheetData>");
    for (r, row) in sheet.rows.iter().enumerate() {
        out.push_str(&format!("<row r=\"{}\">", r + 1));
        for (c, cell) in row.iter().enumerate() {
            let at = format!("{}{}", xml::column(c), r + 1);
            let style = if sheet.header && r == 0 { " s=\"1\"" } else { "" };
            match cell {
                SheetCell::Empty => {
                    if !style.is_empty() {
                        out.push_str(&format!("<c r=\"{at}\"{style}/>"));
                    }
                }
                SheetCell::Text(t) => out.push_str(&format!(
                    "<c r=\"{at}\"{style} t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",
                    xml::escape(t)
                )),
                SheetCell::Number(n) => out.push_str(&format!("<c r=\"{at}\"{style}><v>{}</v></c>", xml::number_text(*n))),
                SheetCell::Bool(b) => out.push_str(&format!("<c r=\"{at}\"{style} t=\"b\"><v>{}</v></c>", u8::from(*b))),
                SheetCell::Formula(f) => out.push_str(&format!("<c r=\"{at}\"{style}><f>{}</f></c>", xml::escape(f))),
            }
        }
        out.push_str("</row>");
    }
    out.push_str("</sheetData>");
    if frozen {
        out.push_str(&format!("<autoFilter ref=\"A1:{}{}\"/>", xml::column(columns - 1), sheet.rows.len()));
    }
    out + "</worksheet>"
}

/// The parts of an .xlsx.
pub fn parts(sheets: &[SheetData]) -> Vec<Part> {
    let count = sheets.len();
    let mut types = String::from(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#,
    );
    for i in 1..=count {
        types.push_str(&format!(
            "<Override PartName=\"/xl/worksheets/sheet{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>"
        ));
    }
    types.push_str("</Types>");
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
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
    let workbook = format!("<workbook xmlns=\"{MAIN}\" xmlns:r=\"{REL}\"><sheets>{sheet_list}</sheets></workbook>");
    let styles = [
        format!("<styleSheet xmlns=\"{MAIN}\">"),
        r#"<fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts>"#.into(),
        r#"<fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFD9E2F3"/><bgColor indexed="64"/></patternFill></fill></fills>"#.into(),
        r#"<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>"#.into(),
        r#"<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>"#.into(),
        r#"<cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="2" borderId="0" xfId="0" applyFont="1" applyFill="1"/></cellXfs>"#.into(),
        r#"<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#.into(),
    ]
    .concat();
    let mut out = vec![
        xml::part("[Content_Types].xml", &types),
        xml::part("_rels/.rels", rels),
        xml::part("xl/workbook.xml", &workbook),
        xml::part("xl/_rels/workbook.xml.rels", &workbook_rels),
        xml::part("xl/styles.xml", &styles),
    ];
    for (i, sheet) in sheets.iter().enumerate() {
        out.push(xml::part(&format!("xl/worksheets/sheet{}.xml", i + 1), &sheet_xml(sheet)));
    }
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
        let sheet = |name: &str| SheetData { name: name.into(), rows: vec![], header: true };
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
            },
            SheetData { name: "Hoja2".into(), rows: vec![vec![SheetCell::Text("x".into())]], header: false },
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
    }
}
