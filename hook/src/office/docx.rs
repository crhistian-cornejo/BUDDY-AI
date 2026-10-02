// Ported from MIKA (MIT, revision d050bc5): apps/macos/Sources/Providers/OfficeFiles.swift (DocxWriter)
//! A Word document (.docx): headings, paragraphs with light Markdown, bullet and numbered lists, tables and page
//! breaks, on A4 with Calibri and Spanish proofing.

use super::xml::{self, Part, Run};

/// One piece of a document, in reading order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocBlock {
    Heading { level: i64, text: String },
    Paragraph(String),
    Bullets(Vec<String>),
    Numbered(Vec<String>),
    Table { header: Vec<String>, rows: Vec<Vec<String>> },
    PageBreak,
}

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

fn run_xml(run: &Run) -> String {
    let mut props = String::new();
    if run.code {
        props.push_str(r#"<w:rFonts w:ascii="Consolas" w:hAnsi="Consolas" w:cs="Consolas"/><w:shd w:val="clear" w:color="auto" w:fill="F2F2F2"/>"#);
    }
    if run.bold {
        props.push_str("<w:b/>");
    }
    if run.italic {
        props.push_str("<w:i/>");
    }
    let props = if props.is_empty() { String::new() } else { format!("<w:rPr>{props}</w:rPr>") };
    format!("<w:r>{props}<w:t xml:space=\"preserve\">{}</w:t></w:r>", xml::escape(&run.text))
}

fn paragraph(text: &str, style: Option<&str>, num_id: Option<usize>, bold: bool, extra: &str) -> String {
    let mut props = String::new();
    if let Some(style) = style {
        props.push_str(&format!("<w:pStyle w:val=\"{style}\"/>"));
    }
    if let Some(num_id) = num_id {
        props.push_str(&format!("<w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"{num_id}\"/></w:numPr>"));
    }
    props.push_str(extra);
    let runs: String = xml::runs(text)
        .into_iter()
        .map(|mut r| {
            r.bold |= bold;
            run_xml(&r)
        })
        .collect();
    let props = if props.is_empty() { String::new() } else { format!("<w:pPr>{props}</w:pPr>") };
    format!("<w:p>{props}{runs}</w:p>")
}

fn table(header: &[String], rows: &[Vec<String>]) -> String {
    let columns = header.len().max(rows.iter().map(Vec::len).max().unwrap_or(0)).max(1);
    let width = 9026 / columns;
    let cell = |text: &str, head: bool| {
        let shade = if head { r#"<w:shd w:val="clear" w:color="auto" w:fill="D9E2F3"/>"# } else { "" };
        format!(
            "<w:tc><w:tcPr><w:tcW w:w=\"{width}\" w:type=\"dxa\"/>{shade}</w:tcPr>{}</w:tc>",
            paragraph(text, None, None, head, r#"<w:spacing w:after="0"/>"#)
        )
    };
    let row = |cells: &[String], head: bool| {
        let mut xml = String::from("<w:tr>");
        if head {
            xml.push_str("<w:trPr><w:tblHeader/></w:trPr>");
        }
        for c in 0..columns {
            xml.push_str(&cell(cells.get(c).map(String::as_str).unwrap_or(""), head));
        }
        xml + "</w:tr>"
    };
    let mut xml = String::from(r#"<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="5000" w:type="pct"/></w:tblPr><w:tblGrid>"#);
    xml.push_str(&format!("<w:gridCol w:w=\"{width}\"/>").repeat(columns));
    xml.push_str("</w:tblGrid>");
    if !header.is_empty() {
        xml.push_str(&row(header, true));
    }
    for r in rows {
        xml.push_str(&row(r, false));
    }
    xml + "</w:tbl><w:p/>"
}

fn heading_style(id: &str, name: &str, size: u32, level: u32) -> String {
    format!(
        "<w:style w:type=\"paragraph\" w:styleId=\"{id}\"><w:name w:val=\"{name}\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before=\"320\" w:after=\"120\"/><w:outlineLvl w:val=\"{level}\"/></w:pPr><w:rPr><w:b/><w:color w:val=\"1F3864\"/><w:sz w:val=\"{size}\"/><w:szCs w:val=\"{size}\"/></w:rPr></w:style>"
    )
}

/// The parts of a .docx.
pub fn parts(title: Option<&str>, blocks: &[DocBlock]) -> Vec<Part> {
    let mut body = String::new();
    let mut numbered_lists = 0usize;
    if let Some(title) = title.filter(|t| !t.is_empty()) {
        body.push_str(&paragraph(title, Some("Title"), None, false, ""));
    }
    for block in blocks {
        match block {
            DocBlock::Heading { level, text } => {
                let style = format!("Heading{}", (*level).clamp(1, 3));
                body.push_str(&paragraph(text, Some(&style), None, false, ""));
            }
            DocBlock::Paragraph(text) => body.push_str(&paragraph(text, None, None, false, "")),
            DocBlock::Bullets(items) => {
                for item in items {
                    body.push_str(&paragraph(item, Some("ListParagraph"), Some(1), false, ""));
                }
            }
            DocBlock::Numbered(items) => {
                // Every numbered list restarts at 1: each gets its own w:num.
                numbered_lists += 1;
                for item in items {
                    body.push_str(&paragraph(item, Some("ListParagraph"), Some(1 + numbered_lists), false, ""));
                }
            }
            DocBlock::Table { header, rows } => body.push_str(&table(header, rows)),
            DocBlock::PageBreak => body.push_str(r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#),
        }
    }
    let document = format!(
        "<w:document xmlns:w=\"{W}\"><w:body>{body}<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"708\" w:footer=\"708\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>"
    );

    let mut nums = String::from(r#"<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#);
    for list in 0..numbered_lists {
        nums.push_str(&format!(
            "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"1\"/><w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"1\"/></w:lvlOverride></w:num>",
            list + 2
        ));
    }
    let numbering = format!(
        "<w:numbering xmlns:w=\"{W}\">{}{}{nums}</w:numbering>",
        r#"<w:abstractNum w:abstractNumId="0"><w:multiLevelType w:val="hybridMultilevel"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="•"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum>"#,
        r#"<w:abstractNum w:abstractNumId="1"><w:multiLevelType w:val="hybridMultilevel"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum>"#,
    );

    let styles = [
        format!("<w:styles xmlns:w=\"{W}\">"),
        r#"<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="Calibri" w:cs="Calibri"/><w:sz w:val="22"/><w:szCs w:val="22"/><w:lang w:val="es-PE"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="120" w:line="276" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults>"#.into(),
        r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>"#.into(),
        r#"<w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:spacing w:after="240"/></w:pPr><w:rPr><w:b/><w:color w:val="1F3864"/><w:sz w:val="48"/><w:szCs w:val="48"/></w:rPr></w:style>"#.into(),
        heading_style("Heading1", "heading 1", 32, 0),
        heading_style("Heading2", "heading 2", 28, 1),
        heading_style("Heading3", "heading 3", 24, 2),
        r#"<w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:spacing w:after="60"/><w:ind w:left="720"/></w:pPr></w:style>"#.into(),
        r#"<w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/><w:uiPriority w:val="99"/><w:semiHidden/><w:tblPr><w:tblInd w:w="0" w:type="dxa"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>"#.into(),
        r#"<w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/><w:basedOn w:val="TableNormal"/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/><w:left w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/><w:bottom w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/><w:right w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/><w:insideH w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/><w:insideV w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/></w:tblBorders></w:tblPr></w:style>"#.into(),
        "</w:styles>".into(),
    ]
    .concat();

    let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/></Types>"#;
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let document_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/></Relationships>"#;
    vec![
        xml::part("[Content_Types].xml", types),
        xml::part("_rels/.rels", rels),
        xml::part("word/document.xml", &document),
        xml::part("word/_rels/document.xml.rels", document_rels),
        xml::part("word/styles.xml", &styles),
        xml::part("word/numbering.xml", &numbering),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(parts: &[Part], path: &str) -> String {
        String::from_utf8(parts.iter().find(|(p, _)| p == path).unwrap().1.clone()).unwrap()
    }

    #[test]
    fn the_document_has_its_parts_styles_and_content() {
        let blocks = vec![
            DocBlock::Heading { level: 1, text: "Uno".into() },
            DocBlock::Heading { level: 9, text: "Hondo".into() },
            DocBlock::Paragraph("a **b** & c <d>".into()),
            DocBlock::Bullets(vec!["x".into(), "y".into()]),
            DocBlock::Numbered(vec!["1".into()]),
            DocBlock::Numbered(vec!["2".into()]),
            DocBlock::Table { header: vec!["A".into(), "B".into()], rows: vec![vec!["1".into()]] },
            DocBlock::PageBreak,
        ];
        let parts = parts(Some("Título \"final\""), &blocks);
        let names: Vec<&str> = parts.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            names,
            [
                "[Content_Types].xml",
                "_rels/.rels",
                "word/document.xml",
                "word/_rels/document.xml.rels",
                "word/styles.xml",
                "word/numbering.xml"
            ]
        );
        let doc = text_of(&parts, "word/document.xml");
        for needle in [
            "w:pStyle w:val=\"Title\"",
            "Título &quot;final&quot;",
            "w:pStyle w:val=\"Heading1\"",
            "w:pStyle w:val=\"Heading3\"",
            " &amp; c &lt;d&gt;",
            "<w:b/></w:rPr><w:t xml:space=\"preserve\">b</w:t>",
            "<w:tbl>",
            "<w:tblHeader/>",
            "w:br w:type=\"page\"",
            "<w:numId w:val=\"1\"/>",
            "<w:numId w:val=\"2\"/>",
            "<w:numId w:val=\"3\"/>",
        ] {
            assert!(doc.contains(needle), "{needle}");
        }
        assert_eq!(doc.matches("<w:tc>").count(), 4, "a short row is padded to the header's width");
        let numbering = text_of(&parts, "word/numbering.xml");
        assert!(numbering.contains("w:numId=\"3\""), "each numbered list restarts");
    }

    #[test]
    fn an_empty_title_is_left_out() {
        let parts = parts(Some(""), &[DocBlock::Paragraph("hola".into())]);
        assert!(!text_of(&parts, "word/document.xml").contains("\"Title\""));
    }
}
