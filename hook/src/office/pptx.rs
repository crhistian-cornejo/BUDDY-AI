// Ported from MIKA (MIT, revision d050bc5): apps/macos/Sources/Providers/OfficeFiles.swift (PptxWriter)
//! A PowerPoint presentation (.pptx), 16:9, on one master with Buddy's look: indigo titles with a mint accent bar,
//! an indigo cover, slide numbers. Slides: cover (title and subtitle), title and bullets (two levels), two columns,
//! picture (alone or beside bullets), each with optional speaker notes.

use super::ListItem;
use super::image::Image;
use super::xml::{self, Part};

/// One column of a two-column slide.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Column {
    pub title: Option<String>,
    pub bullets: Vec<ListItem>,
}

/// A picture on a slide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlideImage {
    pub image: Image,
    pub caption: Option<String>,
}

/// One slide.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlideData {
    pub title: String,
    pub subtitle: Option<String>,
    pub bullets: Vec<ListItem>,
    /// `title` makes a cover-style slide (also good as a section divider) anywhere in the deck.
    pub layout: Option<String>,
    pub left: Option<Column>,
    pub right: Option<Column>,
    pub image: Option<SlideImage>,
    pub notes: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Cover,
    Content,
    TwoColumns,
    Picture,
    PictureAndText,
}

impl Kind {
    fn of(slide: &SlideData, index: usize) -> Kind {
        let layout = slide.layout.as_deref().unwrap_or("");
        if layout == "title" || layout == "section" {
            return Kind::Cover;
        }
        if slide.left.is_some() || slide.right.is_some() {
            return Kind::TwoColumns;
        }
        if slide.image.is_some() {
            return if slide.bullets.is_empty() { Kind::Picture } else { Kind::PictureAndText };
        }
        if index == 0 && slide.bullets.is_empty() && layout.is_empty() {
            return Kind::Cover;
        }
        Kind::Content
    }

    fn layout(self) -> usize {
        match self {
            Kind::Cover => 1,
            Kind::Content | Kind::PictureAndText => 2,
            Kind::TwoColumns => 3,
            Kind::Picture => 4,
        }
    }
}

const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const RELS_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

const INDIGO: &str = "2E3BA8";
const MINT: &str = "1F9D57";
const SLIDE_CX: u64 = 12_192_000;

/// Where things go on a slide (EMU).
#[derive(Clone, Copy)]
struct Rect {
    x: u64,
    y: u64,
    cx: u64,
    cy: u64,
}

const TITLE: Rect = Rect { x: 838_200, y: 365_125, cx: 10_515_600, cy: 1_325_563 };
const BODY: Rect = Rect { x: 838_200, y: 1_825_625, cx: 10_515_600, cy: 4_351_338 };
const LEFT: Rect = Rect { x: 838_200, y: 1_825_625, cx: 5_157_787, cy: 4_351_338 };
const RIGHT: Rect = Rect { x: 6_172_200, y: 1_825_625, cx: 5_181_600, cy: 4_351_338 };
const NUMBER: Rect = Rect { x: 8_610_600, y: 6_356_350, cx: 2_743_200, cy: 365_125 };

fn xfrm(r: Rect) -> String {
    format!("<a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>", r.x, r.y, r.cx, r.cy)
}

fn ns() -> String {
    format!("xmlns:a=\"{A}\" xmlns:r=\"{R}\" xmlns:p=\"{P}\"")
}

const GROUP: &str = r#"<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>"#;

/// The relationships of one slide, gathered while it is written.
struct SlideRels {
    rels: Vec<String>,
}

impl SlideRels {
    fn add(&mut self, kind: &str, target: &str, external: bool) -> String {
        let id = format!("rId{}", self.rels.len() + 1);
        let mode = if external { " TargetMode=\"External\"" } else { "" };
        self.rels.push(format!("<Relationship Id=\"{id}\" Type=\"{REL}/{kind}\" Target=\"{}\"{mode}/>", xml::escape(target)));
        id
    }

    fn xml(&self) -> String {
        format!("<Relationships xmlns=\"{RELS_NS}\">{}</Relationships>", self.rels.concat())
    }
}

fn runs(text: &str, rels: &mut SlideRels, extra: &str, fill: Option<&str>) -> String {
    xml::runs(text)
        .iter()
        .map(|run| {
            let mut attrs = String::from(" lang=\"es-PE\"");
            if run.bold {
                attrs.push_str(" b=\"1\"");
            }
            if run.italic {
                attrs.push_str(" i=\"1\"");
            }
            if run.link.is_some() {
                attrs.push_str(" u=\"sng\"");
            }
            attrs.push_str(extra);
            let mut children = String::new();
            if let Some(color) = fill {
                children.push_str(&format!("<a:solidFill><a:srgbClr val=\"{color}\"/></a:solidFill>"));
            }
            if run.code {
                children.push_str("<a:latin typeface=\"Consolas\"/>");
            }
            if let Some(url) = &run.link {
                children.push_str(&format!("<a:hlinkClick r:id=\"{}\"/>", rels.add("hyperlink", url, true)));
            }
            format!("<a:r><a:rPr{attrs} dirty=\"0\">{children}</a:rPr><a:t>{}</a:t></a:r>", xml::escape(&run.text))
        })
        .collect()
}

/// Bullets on two levels, after an optional bold heading line without a bullet.
fn bullet_paragraphs(heading: Option<&str>, items: &[ListItem], rels: &mut SlideRels) -> String {
    let mut out = String::new();
    if let Some(h) = heading.filter(|h| !h.is_empty()) {
        out.push_str(&format!("<a:p><a:pPr marL=\"0\" indent=\"0\"><a:buNone/></a:pPr>{}</a:p>", runs(h, rels, " b=\"1\"", Some(INDIGO))));
    }
    for item in items {
        out.push_str(&format!("<a:p>{}</a:p>", runs(&item.text, rels, "", None)));
        for child in &item.children {
            out.push_str(&format!("<a:p><a:pPr lvl=\"1\"/>{}</a:p>", runs(child, rels, "", None)));
        }
    }
    if out.is_empty() { "<a:p><a:endParaRPr lang=\"es-PE\"/></a:p>".into() } else { out }
}

/// A rough shrink so long bullets stay on the slide: PowerPoint keeps the `fontScale` we write until the text is
/// edited, when it computes its own.
fn autofit(heading: bool, items: &[ListItem], area: Rect) -> String {
    let width_pt = area.cx as f64 / 12_700.0 - 14.4;
    let height_pt = area.cy as f64 / 12_700.0 - 7.2;
    let lines = |text: &str, size: f64, indent: f64| {
        let per_line = ((width_pt - indent) / (size * 0.5)).max(10.0);
        (xml::plain(text).chars().count() as f64 / per_line).ceil().max(1.0)
    };
    let mut needed = if heading { 24.0 * 1.2 + 6.0 } else { 0.0 };
    for item in items {
        needed += lines(&item.text, 24.0, 27.0) * 24.0 * 1.2 + 6.0;
        for child in &item.children {
            needed += lines(child, 20.0, 54.0) * 20.0 * 1.2 + 6.0;
        }
    }
    if needed <= height_pt {
        return "<a:bodyPr><a:normAutofit/></a:bodyPr>".into();
    }
    // Text area grows with the square of the font size.
    let scale = (height_pt / needed).sqrt().clamp(0.55, 1.0);
    let scale = ((scale * 40.0).floor() / 40.0 * 100_000.0) as u32;
    format!("<a:bodyPr><a:normAutofit fontScale=\"{scale}\" lnSpcReduction=\"10000\"/></a:bodyPr>")
}

fn placeholder(id: u32, name: &str, ph: &str, frame: Option<Rect>, body_pr: &str, paragraphs: &str) -> String {
    let sp_pr = frame.map(|r| format!("<p:spPr>{}</p:spPr>", xfrm(r))).unwrap_or_else(|| "<p:spPr/>".into());
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr>{ph}</p:nvPr></p:nvSpPr>{sp_pr}<p:txBody>{body_pr}<a:lstStyle/>{paragraphs}</p:txBody></p:sp>"
    )
}

fn slide_number(id: u32, n: usize) -> String {
    placeholder(
        id,
        "Número de diapositiva",
        "<p:ph type=\"sldNum\" sz=\"quarter\" idx=\"12\"/>",
        None,
        "<a:bodyPr/>",
        &format!("<a:p><a:fld id=\"{{B6F15528-21DE-4FAA-801E-634DDDAF4B2B}}\" type=\"slidenum\"><a:rPr lang=\"es-PE\"/><a:t>{n}</a:t></a:fld><a:endParaRPr lang=\"es-PE\"/></a:p>"),
    )
}

fn picture(id: u32, picture: &SlideImage, embed: &str, area: Rect) -> String {
    let caption_h = if picture.caption.is_some() { 457_200 } else { 0 };
    let (cx, cy) = picture.image.fit(Some(area.cx), area.cx, area.cy - caption_h);
    let x = area.x + (area.cx - cx) / 2;
    let y = area.y + (area.cy - caption_h - cy) / 2;
    let descr = xml::escape(picture.caption.as_deref().unwrap_or(""));
    let mut out = format!(
        "<p:pic><p:nvPicPr><p:cNvPr id=\"{id}\" name=\"Imagen {id}\" descr=\"{descr}\"/><p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed=\"{embed}\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>",
        xfrm(Rect { x, y, cx, cy })
    );
    if let Some(caption) = &picture.caption {
        let frame = Rect { x: area.x, y: y + cy + 45_720, cx: area.cx, cy: 365_760 };
        out.push_str(&format!(
            "<p:sp><p:nvSpPr><p:cNvPr id=\"{}\" name=\"Pie de imagen\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr><p:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr wrap=\"square\" lIns=\"0\" rIns=\"0\"><a:normAutofit/></a:bodyPr><a:lstStyle/><a:p><a:pPr algn=\"ctr\"/><a:r><a:rPr lang=\"es-PE\" sz=\"1400\" i=\"1\" dirty=\"0\"><a:solidFill><a:srgbClr val=\"595959\"/></a:solidFill></a:rPr><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp>",
            id + 1,
            xfrm(frame),
            xml::escape(&xml::plain(caption))
        ));
    }
    out
}

/// A slide's XML and its relationships. `image` is the media file name when the slide has a picture.
fn slide_xml(slide: &SlideData, kind: Kind, n: usize, image: Option<&str>, notes: Option<usize>) -> (String, String) {
    let mut rels = SlideRels { rels: Vec::new() };
    rels.add("slideLayout", &format!("../slideLayouts/slideLayout{}.xml", kind.layout()), false);
    let title = runs(&slide.title, &mut rels, "", None);
    let title_p = format!("<a:p>{title}</a:p>");
    let mut shapes = String::new();
    match kind {
        Kind::Cover => {
            shapes.push_str(&placeholder(2, "Título 1", "<p:ph type=\"ctrTitle\"/>", None, "<a:bodyPr/>", &title_p));
            if let Some(subtitle) = slide.subtitle.as_ref().filter(|s| !s.is_empty()) {
                let p = format!("<a:p>{}</a:p>", runs(subtitle, &mut rels, "", None));
                shapes.push_str(&placeholder(3, "Subtítulo 2", "<p:ph type=\"subTitle\" idx=\"1\"/>", None, "<a:bodyPr/>", &p));
            }
        }
        _ => {
            shapes.push_str(&placeholder(2, "Título 1", "<p:ph type=\"title\"/>", None, "<a:bodyPr/>", &title_p));
            match kind {
                Kind::Content => {
                    let body = bullet_paragraphs(None, &slide.bullets, &mut rels);
                    shapes.push_str(&placeholder(3, "Contenido 2", "<p:ph idx=\"1\"/>", None, &autofit(false, &slide.bullets, BODY), &body));
                }
                Kind::TwoColumns => {
                    for (i, (column, area)) in [(&slide.left, LEFT), (&slide.right, RIGHT)].into_iter().enumerate() {
                        let column = column.clone().unwrap_or_default();
                        let body = bullet_paragraphs(column.title.as_deref(), &column.bullets, &mut rels);
                        let fit = autofit(column.title.is_some(), &column.bullets, area);
                        let name = if i == 0 { "Contenido izquierdo" } else { "Contenido derecho" };
                        shapes.push_str(&placeholder(3 + i as u32, name, &format!("<p:ph sz=\"half\" idx=\"{}\"/>", i + 1), None, &fit, &body));
                    }
                }
                Kind::PictureAndText => {
                    let body = bullet_paragraphs(None, &slide.bullets, &mut rels);
                    shapes.push_str(&placeholder(3, "Contenido 2", "<p:ph idx=\"1\"/>", Some(LEFT), &autofit(false, &slide.bullets, LEFT), &body));
                }
                _ => {}
            }
            if let (Some(file), Some(pic)) = (image, &slide.image) {
                let embed = rels.add("image", &format!("../media/{file}"), false);
                let area = if kind == Kind::PictureAndText { RIGHT } else { BODY };
                shapes.push_str(&picture(6, pic, &embed, area));
            }
            shapes.push_str(&slide_number(9, n));
        }
    }
    if let Some(notes) = notes {
        rels.add("notesSlide", &format!("../notesSlides/notesSlide{notes}.xml"), false);
    }
    let xml = format!(
        "<p:sld {}><p:cSld><p:spTree>{GROUP}{shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>",
        ns()
    );
    (xml, rels.xml())
}

fn notes_xml(text: &str) -> String {
    let mut rels = SlideRels { rels: Vec::new() };
    let paragraphs: String = text.lines().map(|line| format!("<a:p>{}</a:p>", runs(line, &mut rels, "", None))).collect();
    let paragraphs = if paragraphs.is_empty() { "<a:p><a:endParaRPr lang=\"es-PE\"/></a:p>".to_string() } else { paragraphs };
    format!(
        "<p:notes {}><p:cSld><p:spTree>{GROUP}<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Imagen de diapositiva 1\"/><p:cNvSpPr><a:spLocks noGrp=\"1\" noRot=\"1\" noChangeAspect=\"1\"/></p:cNvSpPr><p:nvPr><p:ph type=\"sldImg\"/></p:nvPr></p:nvSpPr><p:spPr/></p:sp>{}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:notes>",
        ns(),
        placeholder(3, "Notas 2", "<p:ph type=\"body\" idx=\"1\"/>", None, "<a:bodyPr/>", &paragraphs)
    )
}

fn notes_master() -> String {
    format!(
        "<p:notesMaster {}><p:cSld><p:bg><p:bgRef idx=\"1001\"><a:schemeClr val=\"bg1\"/></p:bgRef></p:bg><p:spTree>{GROUP}<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Imagen de diapositiva\"/><p:cNvSpPr><a:spLocks noGrp=\"1\" noRot=\"1\" noChangeAspect=\"1\"/></p:cNvSpPr><p:nvPr><p:ph type=\"sldImg\" idx=\"2\"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"685800\" y=\"1143000\"/><a:ext cx=\"5486400\" cy=\"3086100\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/><a:ln w=\"12700\"><a:solidFill><a:prstClr val=\"black\"/></a:solidFill></a:ln></p:spPr></p:sp><p:sp><p:nvSpPr><p:cNvPr id=\"3\" name=\"Notas\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr><p:ph type=\"body\" sz=\"quarter\" idx=\"3\"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"685800\" y=\"4400550\"/><a:ext cx=\"5486400\" cy=\"3600450\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang=\"es-PE\"/><a:t>Notas</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld>{CLR_MAP}<p:notesStyle><a:lvl1pPr marL=\"0\" algn=\"l\"><a:defRPr sz=\"1200\" kern=\"1200\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mn-lt\"/></a:defRPr></a:lvl1pPr></p:notesStyle></p:notesMaster>",
        ns()
    )
}

const CLR_MAP: &str = "<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>";

/// A placeholder with its own frame and text look, for the master and the layouts.
struct Frame<'a> {
    id: u32,
    name: &'a str,
    ph: &'a str,
    rect: Rect,
    anchor: &'a str,
    size: u32,
    bold: bool,
    align: &'a str,
    color: Option<&'a str>,
    /// No bullet (titles, subtitles, numbers); the master's body keeps the bullets of the text styles.
    plain: bool,
    sample: &'a str,
}

fn shape(f: Frame) -> String {
    let Frame { id, name, ph, rect, anchor, size, bold, align, color, plain, sample } = f;
    let bold = if bold { " b=\"1\"" } else { "" };
    let (margin, bullet) = if plain { (" marL=\"0\" indent=\"0\"", "<a:buNone/>") } else { ("", "") };
    let fill = color.map(|c| format!("<a:solidFill><a:srgbClr val=\"{c}\"/></a:solidFill>")).unwrap_or_default();
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr>{ph}</p:nvPr></p:nvSpPr><p:spPr>{}</p:spPr><p:txBody><a:bodyPr anchor=\"{anchor}\"><a:normAutofit/></a:bodyPr><a:lstStyle><a:lvl1pPr{margin} algn=\"{align}\">{bullet}<a:defRPr sz=\"{size}\"{bold}>{fill}</a:defRPr></a:lvl1pPr></a:lstStyle><a:p><a:r><a:rPr lang=\"es-PE\"/><a:t>{sample}</a:t></a:r></a:p></p:txBody></p:sp>",
        xfrm(rect)
    )
}

/// A plain coloured rectangle (the accent bars).
fn bar(id: u32, name: &str, rect: Rect, color: &str) -> String {
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvSpPr/><p:nvPr userDrawn=\"1\"/></p:nvSpPr><p:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val=\"{color}\"/></a:solidFill><a:ln><a:noFill/></a:ln></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang=\"es-PE\"/></a:p></p:txBody></p:sp>",
        xfrm(rect)
    )
}

fn level_style(tag: &str, size: u32, bullet: Option<&str>, margin: u64, color: Option<&str>, bold: bool) -> String {
    let (mar_l, indent) = if bullet.is_some() { (342_900 + margin, -342_900i64) } else { (0, 0) };
    let bullet = match bullet {
        Some(c) => format!("<a:buClr><a:srgbClr val=\"{MINT}\"/></a:buClr><a:buFont typeface=\"Arial\"/><a:buChar char=\"{c}\"/>"),
        None => "<a:buNone/>".into(),
    };
    let fill = match color {
        Some(c) => format!("<a:srgbClr val=\"{c}\"/>"),
        None => "<a:schemeClr val=\"tx1\"/>".into(),
    };
    let bold = if bold { " b=\"1\"" } else { "" };
    format!(
        "<a:{tag} marL=\"{mar_l}\" indent=\"{indent}\" algn=\"l\"><a:spcBef><a:spcPts val=\"600\"/></a:spcBef>{bullet}<a:defRPr sz=\"{size}\" kern=\"1200\"{bold}><a:solidFill>{fill}</a:solidFill><a:latin typeface=\"+mn-lt\"/></a:defRPr></a:{tag}>"
    )
}

fn layout(ns: &str, kind: &str, name: &str, shapes: &str, extra: &str) -> String {
    format!(
        "<p:sldLayout {ns} type=\"{kind}\" preserve=\"1\"{extra}><p:cSld name=\"{name}\"><p:spTree>{GROUP}{shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"
    )
}

fn theme(name: &str) -> String {
    let fills = "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>".repeat(3);
    [
        format!("<a:theme xmlns:a=\"{A}\" name=\"{name}\"><a:themeElements><a:clrScheme name=\"Buddy\">"),
        r#"<a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="1F2937"/></a:dk2><a:lt2><a:srgbClr val="F3F4F6"/></a:lt2>"#.into(),
        format!("<a:accent1><a:srgbClr val=\"{INDIGO}\"/></a:accent1><a:accent2><a:srgbClr val=\"{MINT}\"/></a:accent2>"),
        r#"<a:accent3><a:srgbClr val="F59E0B"/></a:accent3><a:accent4><a:srgbClr val="0EA5E9"/></a:accent4><a:accent5><a:srgbClr val="E11D48"/></a:accent5><a:accent6><a:srgbClr val="6B7280"/></a:accent6>"#.into(),
        format!("<a:hlink><a:srgbClr val=\"{INDIGO}\"/></a:hlink><a:folHlink><a:srgbClr val=\"6D28D9\"/></a:folHlink>"),
        r#"</a:clrScheme><a:fontScheme name="Buddy"><a:majorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme>"#.into(),
        format!("<a:fmtScheme name=\"Buddy\"><a:fillStyleLst>{fills}</a:fillStyleLst>"),
        format!("<a:lnStyleLst>{}</a:lnStyleLst>", r#"<a:ln w="9525"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>"#.repeat(3)),
        format!("<a:effectStyleLst>{}</a:effectStyleLst>", "<a:effectStyle><a:effectLst/></a:effectStyle>".repeat(3)),
        format!("<a:bgFillStyleLst>{fills}</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"),
    ]
    .concat()
}

/// The parts of a .pptx.
pub fn parts(slides: &[SlideData]) -> Vec<Part> {
    let ns = ns();
    let count = slides.len();
    let kinds: Vec<Kind> = slides.iter().enumerate().map(|(i, s)| Kind::of(s, i)).collect();
    let noted: Vec<bool> = slides.iter().map(|s| s.notes.as_deref().is_some_and(|n| !n.trim().is_empty())).collect();
    let any_notes = noted.iter().any(|n| *n);

    let mut types = String::from(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="jpeg" ContentType="image/jpeg"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/><Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>"#,
    );
    for i in 1..=4 {
        types.push_str(&format!(
            "<Override PartName=\"/ppt/slideLayouts/slideLayout{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>"
        ));
    }
    types.push_str(&format!(
        "<Override PartName=\"/docProps/core.xml\" ContentType=\"{}\"/><Override PartName=\"/docProps/app.xml\" ContentType=\"{}\"/>",
        xml::CORE_TYPE,
        xml::APP_TYPE
    ));
    if any_notes {
        types.push_str(r#"<Override PartName="/ppt/notesMasters/notesMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.notesMaster+xml"/><Override PartName="/ppt/theme/theme2.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>"#);
    }
    for i in 1..=count {
        types.push_str(&format!(
            "<Override PartName=\"/ppt/slides/slide{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"
        ));
        if noted[i - 1] {
            types.push_str(&format!(
                "<Override PartName=\"/ppt/notesSlides/notesSlide{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml\"/>"
            ));
        }
    }
    types.push_str("</Types>");
    let rels = format!(
        "<Relationships xmlns=\"{RELS_NS}\"><Relationship Id=\"rId1\" Type=\"{REL}/officeDocument\" Target=\"ppt/presentation.xml\"/>{}</Relationships>",
        xml::PROPS_RELS
    );

    let mut ids = String::new();
    let mut presentation_rels = SlideRels { rels: Vec::new() };
    presentation_rels.add("slideMaster", "slideMasters/slideMaster1.xml", false);
    for i in 1..=count {
        let id = presentation_rels.add("slide", &format!("slides/slide{i}.xml"), false);
        ids.push_str(&format!("<p:sldId id=\"{}\" r:id=\"{id}\"/>", 255 + i));
    }
    presentation_rels.add("theme", "theme/theme1.xml", false);
    let notes_master_list = if any_notes {
        let id = presentation_rels.add("notesMaster", "notesMasters/notesMaster1.xml", false);
        format!("<p:notesMasterIdLst><p:notesMasterId r:id=\"{id}\"/></p:notesMasterIdLst>")
    } else {
        String::new()
    };
    let presentation = format!(
        "<p:presentation {ns} saveSubsetFonts=\"1\"><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst>{notes_master_list}<p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx=\"12192000\" cy=\"6858000\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>"
    );

    let body_levels: String = (1..=5u32)
        .map(|l| {
            let size = [2400, 2000, 1800, 1600, 1600][l as usize - 1];
            let bullet = if l == 1 { "•" } else { "–" };
            level_style(&format!("lvl{l}pPr"), size, Some(bullet), (l as u64 - 1) * 457_200, None, false)
        })
        .collect();
    let number_frame = |id: u32| {
        shape(Frame { id, name: "Número de diapositiva", ph: "<p:ph type=\"sldNum\" sz=\"quarter\" idx=\"12\"/>", rect: NUMBER, anchor: "ctr", size: 1200, bold: false, align: "r", color: Some("7F7F7F"), plain: true, sample: "‹#›" })
    };
    let master = [
        format!("<p:sldMaster {ns}><p:cSld><p:bg><p:bgRef idx=\"1001\"><a:schemeClr val=\"bg1\"/></p:bgRef></p:bg><p:spTree>{GROUP}"),
        shape(Frame { id: 2, name: "Título 1", ph: "<p:ph type=\"title\"/>", rect: TITLE, anchor: "ctr", size: 3600, bold: true, align: "l", color: Some(INDIGO), plain: true, sample: "Título" }),
        shape(Frame { id: 3, name: "Texto 2", ph: "<p:ph type=\"body\" idx=\"1\"/>", rect: BODY, anchor: "t", size: 2400, bold: false, align: "l", color: None, plain: false, sample: "Texto" }),
        number_frame(4),
        bar(5, "Barra de acento", Rect { x: 457_200, y: 640_080, cx: 91_440, cy: 777_240 }, MINT),
        bar(6, "Banda inferior", Rect { x: 0, y: 6_781_800, cx: SLIDE_CX, cy: 76_200 }, INDIGO),
        format!("</p:spTree></p:cSld>{CLR_MAP}"),
        "<p:sldLayoutIdLst>".into(),
        (1..=4).map(|i| format!("<p:sldLayoutId id=\"{}\" r:id=\"rId{i}\"/>", 2_147_483_648u64 + i)).collect(),
        "</p:sldLayoutIdLst>".into(),
        format!("<p:txStyles><p:titleStyle>{}</p:titleStyle><p:bodyStyle>{body_levels}</p:bodyStyle>", level_style("lvl1pPr", 3600, None, 0, Some(INDIGO), true)),
        format!("<p:otherStyle>{}</p:otherStyle></p:txStyles></p:sldMaster>", level_style("lvl1pPr", 1800, None, 0, None, false)),
    ]
    .concat();
    let mut master_rels = SlideRels { rels: Vec::new() };
    for i in 1..=4 {
        master_rels.add("slideLayout", &format!("../slideLayouts/slideLayout{i}.xml"), false);
    }
    master_rels.add("theme", "../theme/theme1.xml", false);

    let cover_layout = layout(
        &ns,
        "title",
        "Portada",
        &[
            shape(Frame { id: 2, name: "Título 1", ph: "<p:ph type=\"ctrTitle\"/>", rect: Rect { x: 1_524_000, y: 1_122_363, cx: 9_144_000, cy: 2_387_600 }, anchor: "b", size: 4400, bold: true, align: "ctr", color: Some("FFFFFF"), plain: true, sample: "Título" }),
            shape(Frame { id: 3, name: "Subtítulo 2", ph: "<p:ph type=\"subTitle\" idx=\"1\"/>", rect: Rect { x: 1_524_000, y: 3_702_038, cx: 9_144_000, cy: 1_655_762 }, anchor: "t", size: 2000, bold: false, align: "ctr", color: Some("DDE1FF"), plain: true, sample: "Subtítulo" }),
            bar(4, "Barra de acento", Rect { x: (SLIDE_CX - 1_828_800) / 2, y: 3_566_160, cx: 1_828_800, cy: 54_864 }, MINT),
        ]
        .concat(),
        " showMasterSp=\"0\"",
    )
    .replace("<p:cSld name=\"Portada\">", &format!("<p:cSld name=\"Portada\"><p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"{INDIGO}\"/></a:solidFill><a:effectLst/></p:bgPr></p:bg>"));
    let empty = |id: u32, name: &str, ph: &str| {
        format!("<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr>{ph}</p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang=\"es-PE\"/><a:t>{name}</a:t></a:r></a:p></p:txBody></p:sp>")
    };
    let half = |id: u32, name: &str, idx: u32, rect: Rect| {
        format!("<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr><p:ph sz=\"half\" idx=\"{idx}\"/></p:nvPr></p:nvSpPr><p:spPr>{}</p:spPr><p:txBody><a:bodyPr><a:normAutofit/></a:bodyPr><a:lstStyle/><a:p><a:r><a:rPr lang=\"es-PE\"/><a:t>Texto</a:t></a:r></a:p></p:txBody></p:sp>", xfrm(rect))
    };
    let number = "<p:sp><p:nvSpPr><p:cNvPr id=\"9\" name=\"Número de diapositiva\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr><p:ph type=\"sldNum\" sz=\"quarter\" idx=\"12\"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:fld id=\"{B6F15528-21DE-4FAA-801E-634DDDAF4B2B}\" type=\"slidenum\"><a:rPr lang=\"es-PE\"/><a:t>‹#›</a:t></a:fld><a:endParaRPr lang=\"es-PE\"/></a:p></p:txBody></p:sp>";
    let title_ph = empty(2, "Título 1", "<p:ph type=\"title\"/>");
    let content_layout = layout(&ns, "obj", "Título y contenido", &[title_ph.clone(), empty(3, "Contenido 2", "<p:ph idx=\"1\"/>"), number.into()].concat(), "");
    let two_layout = layout(
        &ns,
        "twoObj",
        "Dos objetos",
        &[title_ph.clone(), half(3, "Contenido izquierdo", 1, LEFT), half(4, "Contenido derecho", 2, RIGHT), number.into()].concat(),
        "",
    );
    let title_only = layout(&ns, "titleOnly", "Solo el título", &[title_ph, number.into()].concat(), "");
    let layout_rels = format!("<Relationships xmlns=\"{RELS_NS}\"><Relationship Id=\"rId1\" Type=\"{REL}/slideMaster\" Target=\"../slideMasters/slideMaster1.xml\"/></Relationships>");

    let mut out = vec![
        xml::part("[Content_Types].xml", &types),
        xml::part("_rels/.rels", &rels),
        xml::part("ppt/presentation.xml", &presentation),
        xml::part("ppt/_rels/presentation.xml.rels", &presentation_rels.xml()),
        xml::part("ppt/slideMasters/slideMaster1.xml", &master),
        xml::part("ppt/slideMasters/_rels/slideMaster1.xml.rels", &master_rels.xml()),
        xml::part("ppt/slideLayouts/slideLayout1.xml", &cover_layout),
        xml::part("ppt/slideLayouts/slideLayout2.xml", &content_layout),
        xml::part("ppt/slideLayouts/slideLayout3.xml", &two_layout),
        xml::part("ppt/slideLayouts/slideLayout4.xml", &title_only),
    ];
    for i in 1..=4 {
        out.push(xml::part(&format!("ppt/slideLayouts/_rels/slideLayout{i}.xml.rels"), &layout_rels));
    }
    out.push(xml::part("ppt/theme/theme1.xml", &theme("Buddy")));
    if any_notes {
        out.push(xml::part("ppt/notesMasters/notesMaster1.xml", &notes_master()));
        out.push(xml::part(
            "ppt/notesMasters/_rels/notesMaster1.xml.rels",
            &format!("<Relationships xmlns=\"{RELS_NS}\"><Relationship Id=\"rId1\" Type=\"{REL}/theme\" Target=\"../theme/theme2.xml\"/></Relationships>"),
        ));
        out.push(xml::part("ppt/theme/theme2.xml", &theme("Buddy notas")));
    }
    let mut media = Vec::new();
    for (i, slide) in slides.iter().enumerate() {
        let n = i + 1;
        let kind = kinds[i];
        let image = match (&slide.image, kind) {
            (Some(pic), Kind::Picture | Kind::PictureAndText) => {
                let file = format!("image{}.{}", media.len() + 1, pic.image.ext);
                media.push((format!("ppt/media/{file}"), pic.image.data.clone()));
                Some(file)
            }
            _ => None,
        };
        let (xml, slide_rels) = slide_xml(slide, kind, n, image.as_deref(), noted[i].then_some(n));
        out.push(xml::part(&format!("ppt/slides/slide{n}.xml"), &xml));
        out.push(xml::part(&format!("ppt/slides/_rels/slide{n}.xml.rels"), &slide_rels));
        if noted[i] {
            out.push(xml::part(&format!("ppt/notesSlides/notesSlide{n}.xml"), &notes_xml(slide.notes.as_deref().unwrap_or(""))));
            out.push(xml::part(
                &format!("ppt/notesSlides/_rels/notesSlide{n}.xml.rels"),
                &format!("<Relationships xmlns=\"{RELS_NS}\"><Relationship Id=\"rId1\" Type=\"{REL}/notesMaster\" Target=\"../notesMasters/notesMaster1.xml\"/><Relationship Id=\"rId2\" Type=\"{REL}/slide\" Target=\"../slides/slide{n}.xml\"/></Relationships>"),
            ));
        }
    }
    let title = slides.first().map(|s| xml::plain(&s.title));
    out.extend(xml::properties(title.as_deref(), None, None));
    out.extend(media);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office::image::test_png;

    fn items(texts: &[&str]) -> Vec<ListItem> {
        texts.iter().map(|t| ListItem { text: t.to_string(), children: vec![] }).collect()
    }

    fn text(parts: &[Part], path: &str) -> String {
        String::from_utf8(parts.iter().find(|(p, _)| p == path).unwrap_or_else(|| panic!("{path}")).1.clone()).unwrap()
    }

    #[test]
    fn the_presentation_has_a_cover_and_content_slides() {
        let slides = vec![
            SlideData { title: "Plan & metas".into(), subtitle: Some("2026".into()), ..SlideData::default() },
            SlideData { title: "Metas".into(), bullets: vec![ListItem { text: "a < b".into(), children: vec!["**b**".into()] }], ..SlideData::default() },
        ];
        let parts = parts(&slides);
        let names: Vec<&str> = parts.iter().map(|(p, _)| p.as_str()).collect();
        for name in [
            "[Content_Types].xml",
            "ppt/presentation.xml",
            "ppt/slideMasters/slideMaster1.xml",
            "ppt/slideLayouts/slideLayout1.xml",
            "ppt/slideLayouts/slideLayout4.xml",
            "ppt/theme/theme1.xml",
            "ppt/slides/slide1.xml",
            "ppt/slides/slide2.xml",
            "ppt/slides/_rels/slide2.xml.rels",
            "docProps/core.xml",
        ] {
            assert!(names.contains(&name), "{name}");
        }
        assert!(!names.iter().any(|n| n.contains("notes")), "no notes, no notes master");
        assert!(text(&parts, "ppt/slides/_rels/slide1.xml.rels").contains("slideLayout1.xml"), "the cover uses the cover layout");
        assert!(text(&parts, "ppt/slides/_rels/slide2.xml.rels").contains("slideLayout2.xml"));
        assert!(text(&parts, "ppt/slides/slide1.xml").contains("Plan &amp; metas") && text(&parts, "ppt/slides/slide1.xml").contains("subTitle"));
        let second = text(&parts, "ppt/slides/slide2.xml");
        assert!(second.contains("b=\"1\""));
        assert!(second.contains("a &lt; b"));
        assert!(second.contains("<a:pPr lvl=\"1\"/>"), "second level");
        assert!(second.contains("type=\"slidenum\""), "slide number");
        assert!(text(&parts, "ppt/presentation.xml").contains("<p:sldId id=\"257\" r:id=\"rId3\"/>"));
        let master = text(&parts, "ppt/slideMasters/slideMaster1.xml");
        assert!(master.contains("1F9D57") && master.contains("2E3BA8"), "Buddy's colours");
        assert!(text(&parts, "ppt/slideLayouts/slideLayout1.xml").contains("showMasterSp=\"0\""));
    }

    #[test]
    fn a_first_slide_with_bullets_is_not_a_cover() {
        let parts = parts(&[SlideData { title: "Uno".into(), subtitle: Some("x".into()), bullets: items(&["a"]), ..SlideData::default() }]);
        assert!(text(&parts, "ppt/slides/_rels/slide1.xml.rels").contains("slideLayout2.xml"));
    }

    #[test]
    fn columns_pictures_notes_and_links() {
        let image = SlideImage { image: Image::from_bytes(test_png(300, 150)).unwrap(), caption: Some("Ventas".into()) };
        let slides = vec![
            SlideData {
                title: "Antes y después".into(),
                left: Some(Column { title: Some("Antes".into()), bullets: items(&["lento"]) }),
                right: Some(Column { title: Some("Después".into()), bullets: items(&["rápido [ver](https://buddy.app)"]) }),
                notes: Some("Decir esto\ny esto".into()),
                ..SlideData::default()
            },
            SlideData { title: "Gráfico".into(), image: Some(image.clone()), ..SlideData::default() },
            SlideData { title: "Con texto".into(), image: Some(image), bullets: items(&["punto"]), ..SlideData::default() },
            SlideData { title: "Sección".into(), layout: Some("title".into()), ..SlideData::default() },
        ];
        let parts = parts(&slides);
        let rels = |n: usize| text(&parts, &format!("ppt/slides/_rels/slide{n}.xml.rels"));
        assert!(rels(1).contains("slideLayout3.xml") && rels(1).contains("notesSlide1.xml") && rels(1).contains("https://buddy.app"));
        assert!(rels(2).contains("slideLayout4.xml") && rels(2).contains("../media/image1.png"));
        assert!(rels(3).contains("slideLayout2.xml") && rels(3).contains("../media/image2.png"));
        assert!(rels(4).contains("slideLayout1.xml"), "an explicit title layout is a divider");
        let first = text(&parts, "ppt/slides/slide1.xml");
        assert!(first.contains("idx=\"2\"") && first.contains("Antes") && first.contains("<a:hlinkClick r:id=\"rId2\"/>"));
        assert!(text(&parts, "ppt/slides/slide2.xml").contains("<p:pic>") && text(&parts, "ppt/slides/slide2.xml").contains("Pie de imagen"));
        assert!(text(&parts, "ppt/notesSlides/notesSlide1.xml").contains("Decir esto"));
        assert!(text(&parts, "ppt/presentation.xml").contains("notesMasterIdLst"));
        assert!(text(&parts, "[Content_Types].xml").contains("/ppt/notesSlides/notesSlide1.xml"));
        assert!(!text(&parts, "[Content_Types].xml").contains("notesSlide2.xml"));
        assert!(parts.iter().any(|(p, _)| p == "ppt/media/image2.png"));
    }

    #[test]
    fn long_bullets_shrink() {
        let few = autofit(false, &items(&["corto"]), BODY);
        assert!(!few.contains("fontScale"));
        let many: Vec<String> = (0..20).map(|i| format!("Un punto bastante largo número {i} que ocupa más de una línea en la diapositiva")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let fit = autofit(false, &items(&many), BODY);
        assert!(fit.contains("fontScale=\"55000\"") || fit.contains("fontScale=\"5"), "{fit}");
    }
}
