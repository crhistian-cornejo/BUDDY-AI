// Ported from MIKA (MIT, revision d050bc5): apps/macos/Sources/Providers/OfficeFiles.swift (DocxWriter)
//! A Word document (.docx) on A4 with Calibri and Spanish proofing: a title block (title, subtitle, author, also in
//! the file's properties), an optional table of contents Word fills in when it opens the file, headings with Word's
//! own heading styles (so the navigation pane and the index work), paragraphs with light Markdown and links, bullet
//! and numbered lists with one nested level, tables with a coloured header row, pictures with captions, quotes, code,
//! page breaks, and a header and footer with page numbers. Buddy's indigo and mint are the accents.

use super::ListItem;
use super::image::Image;
use super::xml::{self, Part, Run};

/// One piece of a document, in reading order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocBlock {
    Heading { level: i64, text: String },
    Paragraph(String),
    Bullets(Vec<ListItem>),
    Numbered(Vec<ListItem>),
    /// `widths`: the first columns' widths in twentieths of a point (the rest share what is left).
    Table { header: Vec<String>, rows: Vec<Vec<String>>, widths: Vec<u32> },
    /// `width`: in EMU, when asked.
    Image { image: Image, width: Option<u64>, caption: Option<String> },
    Quote(String),
    Code(String),
    PageBreak,
}

/// Everything about the document that is not a block.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocMeta {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub author: Option<String>,
    /// A table of contents after the title block.
    pub toc: bool,
    pub header: Option<String>,
    pub footer: Option<String>,
    /// "Página N de M" in the footer.
    pub page_numbers: bool,
}

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const WP: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const PIC: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

const INDIGO: &str = "2E3BA8";
const MINT: &str = "1F9D57";
const WHITE: &str = "FFFFFF";
const BAND: &str = "F3F5FB";
/// The width between the margins of an A4 page with 2.54 cm margins, in twentieths of a point.
pub const TEXT_WIDTH: u32 = 9026;
/// The same in EMU (635 per twentieth of a point).
const TEXT_WIDTH_EMU: u64 = TEXT_WIDTH as u64 * 635;
/// The tallest a picture may be: most of the page's text height.
const MAX_PICTURE_HEIGHT_EMU: u64 = 7_200_000;

/// How a run looks beyond its own marks.
#[derive(Clone, Copy, Default)]
struct Look {
    bold: bool,
    color: Option<&'static str>,
}

/// Collects the relationships and media while the body is written.
#[derive(Default)]
struct Builder {
    rels: Vec<String>,
    links: Vec<(String, String)>,
    media: Vec<Part>,
    pictures: usize,
    numbered_lists: usize,
}

impl Builder {
    fn rel(&mut self, kind: &str, target: &str, external: bool) -> String {
        let id = format!("rId{}", self.rels.len() + 1);
        let mode = if external { " TargetMode=\"External\"" } else { "" };
        self.rels.push(format!(
            "<Relationship Id=\"{id}\" Type=\"{REL}/{kind}\" Target=\"{}\"{mode}/>",
            xml::escape(target)
        ));
        id
    }

    fn link(&mut self, url: &str) -> String {
        if let Some((_, id)) = self.links.iter().find(|(u, _)| u == url) {
            return id.clone();
        }
        let id = self.rel("hyperlink", url, true);
        self.links.push((url.to_string(), id.clone()));
        id
    }

    fn run(&mut self, run: &Run, look: Look, links: bool) -> String {
        let mut props = String::new();
        let link = run.link.as_ref().filter(|_| links);
        if link.is_some() {
            props.push_str("<w:rStyle w:val=\"Hyperlink\"/>");
        }
        if run.code {
            props.push_str(r#"<w:rFonts w:ascii="Consolas" w:hAnsi="Consolas" w:cs="Consolas"/>"#);
        }
        if run.bold || look.bold {
            props.push_str("<w:b/>");
        }
        if run.italic {
            props.push_str("<w:i/>");
        }
        if let Some(color) = look.color.filter(|_| link.is_none()) {
            props.push_str(&format!("<w:color w:val=\"{color}\"/>"));
        }
        if run.code {
            props.push_str(r#"<w:sz w:val="20"/><w:szCs w:val="20"/><w:shd w:val="clear" w:color="auto" w:fill="F2F2F2"/>"#);
        }
        let props = if props.is_empty() { String::new() } else { format!("<w:rPr>{props}</w:rPr>") };
        let xml = format!("<w:r>{props}<w:t xml:space=\"preserve\">{}</w:t></w:r>", xml::escape(&run.text));
        match link {
            Some(url) => format!("<w:hyperlink r:id=\"{}\" w:history=\"1\">{xml}</w:hyperlink>", self.link(url)),
            None => xml,
        }
    }

    fn runs(&mut self, text: &str, look: Look, links: bool) -> String {
        xml::runs(text).iter().map(|r| self.run(r, look, links)).collect()
    }

    /// A paragraph. `props` are the paragraph properties after the style, already in the schema's order
    /// (keepNext, numPr, pBdr, shd, tabs, spacing, ind, jc).
    fn paragraph(&mut self, text: &str, style: Option<&str>, props: &str, look: Look) -> String {
        let style = style.map(|s| format!("<w:pStyle w:val=\"{s}\"/>")).unwrap_or_default();
        let ppr = if style.is_empty() && props.is_empty() { String::new() } else { format!("<w:pPr>{style}{props}</w:pPr>") };
        format!("<w:p>{ppr}{}</w:p>", self.runs(text, look, true))
    }

    fn list(&mut self, items: &[ListItem], num_id: usize) -> String {
        let mut out = String::new();
        for item in items {
            let numbering = |level: u8| format!("<w:numPr><w:ilvl w:val=\"{level}\"/><w:numId w:val=\"{num_id}\"/></w:numPr>");
            out.push_str(&self.paragraph(&item.text, Some("ListParagraph"), &numbering(0), Look::default()));
            for child in &item.children {
                out.push_str(&self.paragraph(child, Some("ListParagraph"), &numbering(1), Look::default()));
            }
        }
        out
    }

    fn table(&mut self, header: &[String], rows: &[Vec<String>], widths: &[u32]) -> String {
        let columns = header.len().max(rows.iter().map(Vec::len).max().unwrap_or(0)).max(1);
        let grid = column_widths(header, rows, widths, columns);
        let total: u32 = grid.iter().sum();
        let mut xml = format!(
            "<w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/><w:tblW w:w=\"{total}\" w:type=\"dxa\"/><w:tblLayout w:type=\"fixed\"/><w:tblLook w:val=\"04A0\" w:firstRow=\"1\" w:lastRow=\"0\" w:firstColumn=\"0\" w:lastColumn=\"0\" w:noHBand=\"0\" w:noVBand=\"1\"/></w:tblPr><w:tblGrid>"
        );
        for width in &grid {
            xml.push_str(&format!("<w:gridCol w:w=\"{width}\"/>"));
        }
        xml.push_str("</w:tblGrid>");
        let mut row = |cells: &[String], head: bool, band: bool| {
            let mut out = String::from("<w:tr><w:trPr><w:cantSplit/>");
            if head {
                out.push_str("<w:tblHeader/>");
            }
            out.push_str("</w:trPr>");
            for (c, width) in grid.iter().enumerate() {
                let text = cells.get(c).map(String::as_str).unwrap_or("");
                let fill = if head { Some(INDIGO) } else if band { Some(BAND) } else { None };
                let shade = fill.map(|f| format!("<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{f}\"/>")).unwrap_or_default();
                let align = if !head && numeric(text) { "<w:jc w:val=\"right\"/>" } else { "" };
                let look = if head { Look { bold: true, color: Some(WHITE) } } else { Look::default() };
                let paragraph = self.paragraph(text, None, &format!("<w:spacing w:before=\"40\" w:after=\"40\"/>{align}"), look);
                out.push_str(&format!(
                    "<w:tc><w:tcPr><w:tcW w:w=\"{width}\" w:type=\"dxa\"/>{shade}<w:vAlign w:val=\"center\"/></w:tcPr>{paragraph}</w:tc>"
                ));
            }
            out + "</w:tr>"
        };
        if !header.is_empty() {
            xml.push_str(&row(header, true, false));
        }
        for (i, cells) in rows.iter().enumerate() {
            xml.push_str(&row(cells, false, i % 2 == 1));
        }
        // An empty paragraph after the table, so two tables never fuse and the text does not stick to it.
        xml + "</w:tbl><w:p><w:pPr><w:spacing w:after=\"0\"/></w:pPr></w:p>"
    }

    fn picture(&mut self, image: &Image, width: Option<u64>, caption: Option<&str>) -> String {
        self.pictures += 1;
        let n = self.pictures;
        let file = format!("image{n}.{}", image.ext);
        self.media.push((format!("word/media/{file}"), image.data.clone()));
        let id = self.rel("image", &format!("media/{file}"), false);
        let (cx, cy) = image.fit(width, TEXT_WIDTH_EMU, MAX_PICTURE_HEIGHT_EMU);
        let descr = xml::escape(caption.unwrap_or(""));
        let drawing = format!(
            "<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/><wp:docPr id=\"{n}\" name=\"Imagen {n}\" descr=\"{descr}\"/><wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect=\"1\"/></wp:cNvGraphicFramePr><a:graphic><a:graphicData uri=\"{PIC}\"><pic:pic><pic:nvPicPr><pic:cNvPr id=\"{n}\" name=\"{file}\" descr=\"{descr}\"/><pic:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></pic:cNvPicPr></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{id}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"
        );
        let keep = if caption.is_some() { "<w:keepNext/>" } else { "" };
        let mut out = format!("<w:p><w:pPr>{keep}<w:spacing w:before=\"120\" w:after=\"60\"/><w:jc w:val=\"center\"/></w:pPr>{drawing}</w:p>");
        if let Some(caption) = caption {
            out.push_str(&self.paragraph(caption, Some("Caption"), "", Look::default()));
        }
        out
    }
}

/// A cell that reads as a number, an amount or a percentage: right-aligned like a spreadsheet would.
fn numeric(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty()
        && t.chars().count() <= 25
        && t.chars().any(|c| c.is_ascii_digit())
        && t.chars().all(|c| c.is_ascii_digit() || " .,%+-−$€£S/()".contains(c))
}

/// The width of every column: the ones given (scaled down if they do not fit), the rest shared by how much text
/// each holds.
fn column_widths(header: &[String], rows: &[Vec<String>], given: &[u32], columns: usize) -> Vec<u32> {
    let min = 567u32; // 1 cm
    let mut fixed: Vec<Option<u32>> = (0..columns).map(|c| given.get(c).map(|w| (*w).max(min))).collect();
    let fixed_sum: u32 = fixed.iter().flatten().sum();
    let free = fixed.iter().filter(|w| w.is_none()).count() as u32;
    let budget_for_fixed = TEXT_WIDTH.saturating_sub(free * min);
    if fixed_sum > budget_for_fixed && fixed_sum > 0 {
        for w in fixed.iter_mut().flatten() {
            *w = (*w as u64 * budget_for_fixed as u64 / fixed_sum as u64).max(min as u64) as u32;
        }
    }
    let used: u32 = fixed.iter().flatten().sum();
    let rest = TEXT_WIDTH.saturating_sub(used).max(free * min);
    let weight = |c: usize| {
        let longest = std::iter::once(header).chain(rows.iter().map(Vec::as_slice))
            .filter_map(|r| r.get(c))
            .map(|t| xml::plain(t).chars().count())
            .max()
            .unwrap_or(0);
        longest.clamp(4, 40) as u32
    };
    let weights: Vec<u32> = (0..columns).map(|c| if fixed[c].is_none() { weight(c) } else { 0 }).collect();
    let total_weight: u32 = weights.iter().sum::<u32>().max(1);
    (0..columns)
        .map(|c| fixed[c].unwrap_or_else(|| (rest as u64 * weights[c] as u64 / total_weight as u64).max(min as u64) as u32))
        .collect()
}

fn toc(builder: &mut Builder, blocks: &[DocBlock]) -> String {
    let mut out = builder.paragraph("Contenido", Some("TOCHeading"), "", Look::default());
    let entries: Vec<(i64, String)> = blocks
        .iter()
        .filter_map(|b| match b {
            DocBlock::Heading { level, text } => Some(((*level).clamp(1, 3), xml::plain(text))),
            _ => None,
        })
        .collect();
    let entries = if entries.is_empty() {
        vec![(1, "Haz clic derecho aquí y elige «Actualizar campo» para ver el índice.".to_string())]
    } else {
        entries
    };
    let begin = r#"<w:r><w:fldChar w:fldCharType="begin" w:dirty="true"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \o "1-3" \h \z \u </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>"#;
    let end = r#"<w:r><w:fldChar w:fldCharType="end"/></w:r>"#;
    let last = entries.len() - 1;
    for (i, (level, text)) in entries.iter().enumerate() {
        out.push_str(&format!(
            "<w:p><w:pPr><w:pStyle w:val=\"TOC{level}\"/></w:pPr>{}<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>{}</w:p>",
            if i == 0 { begin } else { "" },
            xml::escape(text),
            if i == last { end } else { "" },
        ));
    }
    out + r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#
}

fn style(kind: &str, id: &str, name: &str, based_on: Option<&str>, ppr: &str, rpr: &str) -> String {
    let based = based_on.map(|b| format!("<w:basedOn w:val=\"{b}\"/>")).unwrap_or_default();
    let next = if kind == "paragraph" { "<w:next w:val=\"Normal\"/>" } else { "" };
    let ppr = if ppr.is_empty() { String::new() } else { format!("<w:pPr>{ppr}</w:pPr>") };
    let rpr = if rpr.is_empty() { String::new() } else { format!("<w:rPr>{rpr}</w:rPr>") };
    format!("<w:style w:type=\"{kind}\" w:styleId=\"{id}\"><w:name w:val=\"{name}\"/>{based}{next}<w:qFormat/>{ppr}{rpr}</w:style>")
}

fn styles() -> String {
    let size = |half_points: u32| format!("<w:sz w:val=\"{half_points}\"/><w:szCs w:val=\"{half_points}\"/>");
    let rule = |side: &str, sz: u32, color: &str, space: u32| {
        format!("<w:pBdr><w:{side} w:val=\"single\" w:sz=\"{sz}\" w:space=\"{space}\" w:color=\"{color}\"/></w:pBdr>")
    };
    let heading = |level: u32, half_points: u32, color: &str, border: bool| {
        style(
            "paragraph",
            &format!("Heading{level}"),
            &format!("heading {level}"),
            Some("Normal"),
            &format!(
                "<w:keepNext/><w:keepLines/>{}<w:spacing w:before=\"{}\" w:after=\"120\"/><w:outlineLvl w:val=\"{}\"/>",
                if border { rule("bottom", 6, MINT, 4) } else { String::new() },
                if level == 1 { 360 } else { 240 },
                level - 1
            ),
            &format!("<w:b/><w:color w:val=\"{color}\"/>{}", size(half_points)),
        )
    };
    let toc_entry = |level: u32| {
        style(
            "paragraph",
            &format!("TOC{level}"),
            &format!("toc {level}"),
            Some("Normal"),
            &format!(
                "<w:tabs><w:tab w:val=\"right\" w:leader=\"dot\" w:pos=\"{TEXT_WIDTH}\"/></w:tabs><w:spacing w:after=\"60\"/><w:ind w:left=\"{}\"/>",
                (level - 1) * 240
            ),
            if level == 1 { "<w:b/>" } else { "" },
        )
    };
    let tabs = format!("<w:tabs><w:tab w:val=\"center\" w:pos=\"{}\"/><w:tab w:val=\"right\" w:pos=\"{TEXT_WIDTH}\"/></w:tabs><w:spacing w:after=\"0\"/>", TEXT_WIDTH / 2);
    [
        format!("<w:styles xmlns:w=\"{W}\">"),
        r#"<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="Calibri" w:cs="Calibri"/><w:sz w:val="22"/><w:szCs w:val="22"/><w:lang w:val="es-PE" w:eastAsia="en-US" w:bidi="ar-SA"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="120" w:line="276" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults>"#.into(),
        r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>"#.into(),
        r#"<w:style w:type="character" w:default="1" w:styleId="DefaultParagraphFont"><w:name w:val="Default Paragraph Font"/><w:uiPriority w:val="1"/><w:semiHidden/><w:unhideWhenUsed/></w:style>"#.into(),
        style("paragraph", "Title", "Title", Some("Normal"), &format!("{}<w:spacing w:after=\"160\" w:line=\"240\" w:lineRule=\"auto\"/>", rule("bottom", 12, MINT, 6)), &format!("<w:b/><w:color w:val=\"{INDIGO}\"/><w:kern w:val=\"28\"/>{}", size(56))),
        style("paragraph", "Subtitle", "Subtitle", Some("Normal"), "<w:spacing w:after=\"120\"/>", &format!("<w:color w:val=\"595959\"/>{}", size(28))),
        style("paragraph", "Author", "Autor", Some("Normal"), "<w:spacing w:after=\"360\"/>", &format!("<w:i/><w:color w:val=\"7F7F7F\"/>{}", size(20))),
        heading(1, 32, INDIGO, true),
        heading(2, 26, INDIGO, false),
        heading(3, 23, "1F2937", false),
        style("paragraph", "TOCHeading", "TOC Heading", Some("Normal"), "<w:keepNext/><w:spacing w:before=\"240\" w:after=\"120\"/>", &format!("<w:b/><w:color w:val=\"{INDIGO}\"/>{}", size(32))),
        toc_entry(1),
        toc_entry(2),
        toc_entry(3),
        style("paragraph", "ListParagraph", "List Paragraph", Some("Normal"), "<w:spacing w:after=\"60\"/><w:ind w:left=\"720\"/><w:contextualSpacing/>", ""),
        style("paragraph", "Quote", "Quote", Some("Normal"), &format!("{}<w:spacing w:before=\"120\" w:after=\"120\"/><w:ind w:left=\"567\" w:right=\"567\"/>", rule("left", 18, MINT, 8)), "<w:i/><w:color w:val=\"404040\"/>"),
        style("paragraph", "Code", "Código", Some("Normal"), "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"F3F4F6\"/><w:spacing w:after=\"120\" w:line=\"240\" w:lineRule=\"auto\"/><w:ind w:left=\"142\" w:right=\"142\"/>", &format!("<w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\" w:cs=\"Consolas\"/>{}", size(19))),
        style("paragraph", "Caption", "caption", Some("Normal"), "<w:spacing w:after=\"200\"/><w:jc w:val=\"center\"/>", &format!("<w:i/><w:color w:val=\"595959\"/>{}", size(18))),
        style("paragraph", "Header", "header", Some("Normal"), &tabs, &format!("<w:color w:val=\"7F7F7F\"/>{}", size(18))),
        style("paragraph", "Footer", "footer", Some("Normal"), &tabs, &format!("<w:color w:val=\"7F7F7F\"/>{}", size(18))),
        style("character", "Hyperlink", "Hyperlink", Some("DefaultParagraphFont"), "", &format!("<w:color w:val=\"{INDIGO}\"/><w:u w:val=\"single\"/>")),
        r#"<w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/><w:uiPriority w:val="99"/><w:semiHidden/><w:tblPr><w:tblInd w:w="0" w:type="dxa"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>"#.into(),
        r#"<w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/><w:basedOn w:val="TableNormal"/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/><w:left w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/><w:bottom w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/><w:right w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/><w:insideH w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/><w:insideV w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/></w:tblBorders></w:tblPr></w:style>"#.into(),
        "</w:styles>".into(),
    ]
    .concat()
}

fn numbering(numbered_lists: usize) -> String {
    let level = |ilvl: u32, format: &str, text: &str, left: u32| {
        format!(
            "<w:lvl w:ilvl=\"{ilvl}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{format}\"/><w:lvlText w:val=\"{text}\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{left}\" w:hanging=\"360\"/></w:pPr></w:lvl>"
        )
    };
    let mut nums = String::from(r#"<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#);
    for list in 0..numbered_lists {
        // Every numbered list restarts at 1: each gets its own w:num.
        nums.push_str(&format!(
            "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"1\"/><w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"1\"/></w:lvlOverride></w:num>",
            list + 2
        ));
    }
    format!(
        "<w:numbering xmlns:w=\"{W}\"><w:abstractNum w:abstractNumId=\"0\"><w:multiLevelType w:val=\"hybridMultilevel\"/>{}{}</w:abstractNum><w:abstractNum w:abstractNumId=\"1\"><w:multiLevelType w:val=\"hybridMultilevel\"/>{}{}</w:abstractNum>{nums}</w:numbering>",
        level(0, "bullet", "•", 720),
        level(1, "bullet", "–", 1440),
        level(0, "decimal", "%1.", 720),
        level(1, "lowerLetter", "%2)", 1440),
    )
}

/// The parts of a .docx.
pub fn parts(meta: &DocMeta, blocks: &[DocBlock]) -> Vec<Part> {
    let mut b = Builder::default();
    b.rel("styles", "styles.xml", false);
    b.rel("numbering", "numbering.xml", false);
    b.rel("settings", "settings.xml", false);
    let header = meta.header.as_deref().filter(|h| !h.trim().is_empty());
    let footer_text = meta.footer.as_deref().filter(|f| !f.trim().is_empty());
    let has_footer = footer_text.is_some() || meta.page_numbers;
    let header_id = header.map(|_| b.rel("header", "header1.xml", false));
    let footer_id = has_footer.then(|| b.rel("footer", "footer1.xml", false));

    let mut body = String::new();
    let none = Look::default();
    if let Some(title) = meta.title.as_deref().filter(|t| !t.is_empty()) {
        body.push_str(&b.paragraph(title, Some("Title"), "", none));
    }
    if let Some(subtitle) = meta.subtitle.as_deref().filter(|t| !t.is_empty()) {
        body.push_str(&b.paragraph(subtitle, Some("Subtitle"), "", none));
    }
    if let Some(author) = meta.author.as_deref().filter(|t| !t.is_empty()) {
        body.push_str(&b.paragraph(author, Some("Author"), "", none));
    }
    if meta.toc {
        body.push_str(&toc(&mut b, blocks));
    }
    for block in blocks {
        let xml = match block {
            DocBlock::Heading { level, text } => b.paragraph(text, Some(&format!("Heading{}", (*level).clamp(1, 3))), "", none),
            DocBlock::Paragraph(text) => b.paragraph(text, None, "", none),
            DocBlock::Bullets(items) => b.list(items, 1),
            DocBlock::Numbered(items) => {
                b.numbered_lists += 1;
                let id = 1 + b.numbered_lists;
                b.list(items, id)
            }
            DocBlock::Table { header, rows, widths } => b.table(header, rows, widths),
            DocBlock::Image { image, width, caption } => b.picture(image, *width, caption.as_deref()),
            DocBlock::Quote(text) => b.paragraph(text, Some("Quote"), "", none),
            DocBlock::Code(text) => {
                let lines: Vec<String> = text
                    .lines()
                    .map(|line| format!("<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>", xml::escape(&line.replace('\t', "    "))))
                    .collect();
                format!("<w:p><w:pPr><w:pStyle w:val=\"Code\"/></w:pPr>{}</w:p>", lines.join("<w:r><w:br/></w:r>"))
            }
            DocBlock::PageBreak => r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#.to_string(),
        };
        body.push_str(&xml);
    }
    let mut section = String::new();
    if let Some(id) = &header_id {
        section.push_str(&format!("<w:headerReference w:type=\"default\" r:id=\"{id}\"/>"));
    }
    if let Some(id) = &footer_id {
        section.push_str(&format!("<w:footerReference w:type=\"default\" r:id=\"{id}\"/>"));
    }
    let document = format!(
        "<w:document xmlns:w=\"{W}\" xmlns:r=\"{R}\" xmlns:wp=\"{WP}\" xmlns:a=\"{A}\" xmlns:pic=\"{PIC}\"><w:body>{body}<w:sectPr>{section}<w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"708\" w:footer=\"708\" w:gutter=\"0\"/><w:cols w:space=\"708\"/></w:sectPr></w:body></w:document>"
    );

    let mut parts_list = Vec::new();
    let mut overrides = vec![
        ("/word/document.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"),
        ("/word/styles.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"),
        ("/word/numbering.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"),
        ("/word/settings.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"),
        ("/docProps/core.xml", xml::CORE_TYPE),
        ("/docProps/app.xml", xml::APP_TYPE),
    ];
    if let Some(text) = header {
        let runs = b.runs(text, none, false);
        parts_list.push(xml::part(
            "word/header1.xml",
            &format!("<w:hdr xmlns:w=\"{W}\" xmlns:r=\"{R}\"><w:p><w:pPr><w:pStyle w:val=\"Header\"/><w:jc w:val=\"right\"/></w:pPr>{runs}</w:p></w:hdr>"),
        ));
        overrides.push(("/word/header1.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"));
    }
    if has_footer {
        let mut runs = footer_text.map(|t| b.runs(t, none, false)).unwrap_or_default();
        if meta.page_numbers {
            runs.push_str(r#"<w:r><w:tab/></w:r><w:r><w:tab/></w:r><w:r><w:t xml:space="preserve">Página </w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r><w:t>1</w:t></w:r></w:fldSimple><w:r><w:t xml:space="preserve"> de </w:t></w:r><w:fldSimple w:instr=" NUMPAGES "><w:r><w:t>1</w:t></w:r></w:fldSimple>"#);
        }
        parts_list.push(xml::part(
            "word/footer1.xml",
            &format!("<w:ftr xmlns:w=\"{W}\" xmlns:r=\"{R}\"><w:p><w:pPr><w:pStyle w:val=\"Footer\"/></w:pPr>{runs}</w:p></w:ftr>"),
        ));
        overrides.push(("/word/footer1.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"));
    }

    let mut types = String::from(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="jpeg" ContentType="image/jpeg"/>"#,
    );
    for (name, kind) in &overrides {
        types.push_str(&format!("<Override PartName=\"{name}\" ContentType=\"{kind}\"/>"));
    }
    types.push_str("</Types>");
    let rels = format!(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{REL}/officeDocument\" Target=\"word/document.xml\"/>{}</Relationships>",
        xml::PROPS_RELS
    );
    let document_rels = format!("<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{}</Relationships>", b.rels.concat());
    let settings = format!(
        "<w:settings xmlns:w=\"{W}\"><w:zoom w:percent=\"100\"/><w:defaultTabStop w:val=\"708\"/><w:characterSpacingControl w:val=\"doNotCompress\"/>{}<w:compat><w:compatSetting w:name=\"compatibilityMode\" w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"15\"/></w:compat></w:settings>",
        if meta.toc { "<w:updateFields w:val=\"true\"/>" } else { "" }
    );

    let mut out = vec![
        xml::part("[Content_Types].xml", &types),
        xml::part("_rels/.rels", &rels),
        xml::part("word/document.xml", &document),
        xml::part("word/_rels/document.xml.rels", &document_rels),
        xml::part("word/styles.xml", &styles()),
        xml::part("word/numbering.xml", &numbering(b.numbered_lists)),
        xml::part("word/settings.xml", &settings),
    ];
    out.extend(parts_list);
    out.extend(xml::properties(meta.title.as_deref(), meta.subtitle.as_deref(), meta.author.as_deref()));
    out.append(&mut b.media);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office::image::test_png;

    fn text_of(parts: &[Part], path: &str) -> String {
        String::from_utf8(parts.iter().find(|(p, _)| p == path).unwrap_or_else(|| panic!("{path}")).1.clone()).unwrap()
    }

    fn items(texts: &[&str]) -> Vec<ListItem> {
        texts.iter().map(|t| ListItem { text: t.to_string(), children: vec![] }).collect()
    }

    #[test]
    fn the_document_has_its_parts_styles_and_content() {
        let blocks = vec![
            DocBlock::Heading { level: 1, text: "Uno".into() },
            DocBlock::Heading { level: 9, text: "Hondo".into() },
            DocBlock::Paragraph("a **b** & c <d> [web](https://buddy.app)".into()),
            DocBlock::Bullets(vec![ListItem { text: "x".into(), children: vec!["x.1".into()] }]),
            DocBlock::Numbered(items(&["1"])),
            DocBlock::Numbered(items(&["2"])),
            DocBlock::Table { header: vec!["A".into(), "B".into()], rows: vec![vec!["1".into()]], widths: vec![] },
            DocBlock::Quote("cita".into()),
            DocBlock::Code("fn main() {\n\tprint!(\"<x>\");\n}".into()),
            DocBlock::PageBreak,
        ];
        let meta = DocMeta { title: Some("Título \"final\"".into()), page_numbers: true, ..DocMeta::default() };
        let parts = parts(&meta, &blocks);
        let names: Vec<&str> = parts.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            names,
            [
                "[Content_Types].xml",
                "_rels/.rels",
                "word/document.xml",
                "word/_rels/document.xml.rels",
                "word/styles.xml",
                "word/numbering.xml",
                "word/settings.xml",
                "word/footer1.xml",
                "docProps/core.xml",
                "docProps/app.xml",
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
            "<w:hyperlink r:id=\"rId5\" w:history=\"1\">",
            "<w:tbl>",
            "<w:tblHeader/>",
            "w:br w:type=\"page\"",
            "<w:ilvl w:val=\"1\"/><w:numId w:val=\"1\"/>",
            "<w:numId w:val=\"2\"/>",
            "<w:numId w:val=\"3\"/>",
            "w:pStyle w:val=\"Quote\"",
            "    print!(&quot;&lt;x&gt;&quot;);",
            "<w:footerReference w:type=\"default\" r:id=\"rId4\"/>",
        ] {
            assert!(doc.contains(needle), "{needle}");
        }
        assert_eq!(doc.matches("<w:tc>").count(), 4, "a short row is padded to the header's width");
        let rels = text_of(&parts, "word/_rels/document.xml.rels");
        assert!(rels.contains("Target=\"https://buddy.app\" TargetMode=\"External\""));
        assert!(text_of(&parts, "word/numbering.xml").contains("w:numId=\"3\""), "each numbered list restarts");
        assert!(text_of(&parts, "word/footer1.xml").contains("NUMPAGES"));
        assert!(text_of(&parts, "docProps/core.xml").contains("<dc:title>Título &quot;final&quot;</dc:title>"));
        let styles = text_of(&parts, "word/styles.xml");
        for id in ["Heading1", "Heading2", "Heading3", "TOC1", "Quote", "Code", "Caption", "Hyperlink", "Footer"] {
            assert!(styles.contains(&format!("w:styleId=\"{id}\"")), "{id}");
        }
        assert!(styles.contains("<w:name w:val=\"heading 1\"/>"), "Word knows its own heading style by name");
    }

    #[test]
    fn title_block_toc_header_and_pictures() {
        let image = Image::from_bytes(test_png(800, 400)).unwrap();
        let blocks = vec![
            DocBlock::Heading { level: 1, text: "Introducción".into() },
            DocBlock::Heading { level: 2, text: "**Alcance**".into() },
            DocBlock::Image { image: image.clone(), width: Some(3_600_000), caption: Some("Figura 1".into()) },
            DocBlock::Image { image, width: None, caption: None },
        ];
        let meta = DocMeta {
            title: Some("Informe".into()),
            subtitle: Some("Q3".into()),
            author: Some("Ana".into()),
            toc: true,
            header: Some("Confidencial".into()),
            footer: None,
            page_numbers: false,
        };
        let parts = parts(&meta, &blocks);
        let doc = text_of(&parts, "word/document.xml");
        assert!(doc.contains("TOC \\o &quot;1-3&quot;") || doc.contains("TOC \\o \"1-3\""));
        assert!(doc.contains("<w:pStyle w:val=\"TOC2\"/></w:pPr><w:r><w:t xml:space=\"preserve\">Alcance</w:t>"), "entries are plain text");
        assert!(doc.contains("w:pStyle w:val=\"Subtitle\"") && doc.contains("w:pStyle w:val=\"Author\""));
        assert!(doc.contains("<wp:extent cx=\"3600000\" cy=\"1800000\"/>"));
        assert!(doc.contains("<wp:extent cx=\"5731510\""), "a wide picture is shrunk to the text width");
        assert!(doc.contains("w:pStyle w:val=\"Caption\""));
        assert!(doc.contains("<w:headerReference w:type=\"default\""));
        assert!(!doc.contains("footerReference"), "no footer asked");
        assert!(text_of(&parts, "word/settings.xml").contains("updateFields"));
        assert!(text_of(&parts, "word/header1.xml").contains("Confidencial"));
        assert!(parts.iter().any(|(p, d)| p == "word/media/image1.png" && d.starts_with(b"\x89PNG")));
        assert!(parts.iter().any(|(p, _)| p == "word/media/image2.png"));
        let rels = text_of(&parts, "word/_rels/document.xml.rels");
        assert!(rels.contains("Target=\"media/image1.png\""));
        assert!(text_of(&parts, "docProps/core.xml").contains("<dc:creator>Ana</dc:creator>"));
    }

    #[test]
    fn column_widths_follow_the_content_and_the_request() {
        let header = vec!["Nombre".to_string(), "Descripción larga".to_string(), "N".to_string()];
        let auto = column_widths(&header, &[], &[], 3);
        assert!(auto[1] > auto[0] && auto[0] > auto[2] - 1);
        assert!(auto.iter().sum::<u32>() <= TEXT_WIDTH + 3);
        let given = column_widths(&header, &[], &[2268], 3);
        assert_eq!(given[0], 2268);
        let too_wide = column_widths(&header, &[], &[9000, 9000, 9000], 3);
        assert!(too_wide.iter().sum::<u32>() <= TEXT_WIDTH + 3);
        assert!(numeric("S/ 1,200.50") && numeric("25%") && numeric("-3") && !numeric("abc") && !numeric(""));
    }

    #[test]
    fn an_empty_title_is_left_out() {
        let parts = parts(&DocMeta { title: Some(String::new()), ..DocMeta::default() }, &[DocBlock::Paragraph("hola".into())]);
        assert!(!text_of(&parts, "word/document.xml").contains("\"Title\""));
        assert!(!parts.iter().any(|(p, _)| p == "word/footer1.xml"));
    }
}
