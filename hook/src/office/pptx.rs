// Ported from MIKA (MIT, revision d050bc5): apps/macos/Sources/Providers/OfficeFiles.swift (PptxWriter)
//! A PowerPoint presentation (.pptx), 16:9: a cover slide (title and subtitle) and title-and-bullets slides, on one
//! master with two layouts and a plain theme.

use super::xml::{self, Part};

/// One slide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlideData {
    pub title: String,
    pub subtitle: Option<String>,
    pub bullets: Vec<String>,
}

const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";

fn ns() -> String {
    format!("xmlns:a=\"{A}\" xmlns:r=\"{R}\" xmlns:p=\"{P}\"")
}

const GROUP: &str = r#"<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>"#;

fn paragraphs(lines: &[String]) -> String {
    lines
        .iter()
        .map(|line| {
            let runs: String = xml::runs(line)
                .iter()
                .map(|run| {
                    format!(
                        "<a:r><a:rPr lang=\"es-PE\"{}{}/><a:t>{}</a:t></a:r>",
                        if run.bold { " b=\"1\"" } else { "" },
                        if run.italic { " i=\"1\"" } else { "" },
                        xml::escape(&run.text)
                    )
                })
                .collect();
            format!("<a:p>{runs}</a:p>")
        })
        .collect()
}

fn placeholder(id: u32, name: &str, kind: Option<&str>, idx: Option<u32>, lines: &[String]) -> String {
    let mut ph = String::from("<p:ph");
    if let Some(kind) = kind {
        ph.push_str(&format!(" type=\"{kind}\""));
    }
    if let Some(idx) = idx {
        ph.push_str(&format!(" idx=\"{idx}\""));
    }
    ph.push_str("/>");
    let body = if lines.is_empty() { "<a:p><a:endParaRPr lang=\"es-PE\"/></a:p>".to_string() } else { paragraphs(lines) };
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr>{ph}</p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/>{body}</p:txBody></p:sp>"
    )
}

fn slide_xml(slide: &SlideData, cover: bool) -> String {
    let title = std::slice::from_ref(&slide.title);
    let mut shapes = placeholder(2, "Título 1", Some(if cover { "ctrTitle" } else { "title" }), None, title);
    if cover {
        if let Some(subtitle) = slide.subtitle.as_ref().filter(|s| !s.is_empty()) {
            shapes.push_str(&placeholder(3, "Subtítulo 2", Some("subTitle"), Some(1), std::slice::from_ref(subtitle)));
        }
    } else {
        shapes.push_str(&placeholder(3, "Contenido 2", None, Some(1), &slide.bullets));
    }
    format!(
        "<p:sld {}><p:cSld><p:spTree>{GROUP}{shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>",
        ns()
    )
}

/// A placeholder with its own frame and text style, for the master and the cover layout.
struct Frame<'a> {
    id: u32,
    name: &'a str,
    ph: &'a str,
    x: u64,
    y: u64,
    cx: u64,
    cy: u64,
    anchor: &'a str,
    size: u32,
    bold: bool,
    align: &'a str,
    sample: &'a str,
}

fn shape(f: Frame) -> String {
    let Frame { id, name, ph, x, y, cx, cy, anchor, size, bold, align, sample } = f;
    let bold = if bold { " b=\"1\"" } else { "" };
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr>{ph}</p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm></p:spPr><p:txBody><a:bodyPr anchor=\"{anchor}\"><a:normAutofit/></a:bodyPr><a:lstStyle><a:lvl1pPr algn=\"{align}\"><a:defRPr sz=\"{size}\"{bold}/></a:lvl1pPr></a:lstStyle><a:p><a:r><a:rPr lang=\"es-PE\"/><a:t>{sample}</a:t></a:r></a:p></p:txBody></p:sp>"
    )
}

fn level_style(tag: &str, size: u32, bullet: bool, margin: u64) -> String {
    let (mar_l, indent) = if bullet { (342_900 + margin, -342_900) } else { (0, 0) };
    let bullet = if bullet { "<a:buFont typeface=\"Arial\"/><a:buChar char=\"•\"/>" } else { "<a:buNone/>" };
    format!(
        "<a:{tag} marL=\"{mar_l}\" indent=\"{indent}\" algn=\"l\"><a:spcBef><a:spcPts val=\"600\"/></a:spcBef>{bullet}<a:defRPr sz=\"{size}\" kern=\"1200\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mn-lt\"/></a:defRPr></a:{tag}>"
    )
}

/// The parts of a .pptx. The first slide is the cover when it has no bullets.
pub fn parts(slides: &[SlideData]) -> Vec<Part> {
    let ns = ns();
    let count = slides.len();
    let mut types = String::from(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/><Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/><Override PartName="/ppt/slideLayouts/slideLayout2.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/><Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>"#,
    );
    for i in 1..=count {
        types.push_str(&format!(
            "<Override PartName=\"/ppt/slides/slide{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"
        ));
    }
    types.push_str("</Types>");
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#;

    let mut ids = String::new();
    let mut presentation_rels = String::from(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>"#,
    );
    for i in 1..=count {
        ids.push_str(&format!("<p:sldId id=\"{}\" r:id=\"rId{}\"/>", 255 + i, i + 1));
        presentation_rels.push_str(&format!(
            "<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide{i}.xml\"/>",
            i + 1
        ));
    }
    presentation_rels.push_str(&format!(
        "<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"theme/theme1.xml\"/></Relationships>",
        count + 2
    ));
    let presentation = format!(
        "<p:presentation {ns}><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst><p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx=\"12192000\" cy=\"6858000\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>"
    );

    let body_levels: String = (1..=5u32)
        .map(|l| level_style(&format!("lvl{l}pPr"), 2800u32.saturating_sub((l - 1) * 400).max(1800), true, (l as u64 - 1) * 342_900))
        .collect();
    let master = [
        format!("<p:sldMaster {ns}><p:cSld><p:bg><p:bgRef idx=\"1001\"><a:schemeClr val=\"bg1\"/></p:bgRef></p:bg><p:spTree>{GROUP}"),
        shape(Frame { id: 2, name: "Título 1", ph: "<p:ph type=\"title\"/>", x: 838_200, y: 365_125, cx: 10_515_600, cy: 1_325_563, anchor: "ctr", size: 4400, bold: true, align: "l", sample: "Título" }),
        shape(Frame { id: 3, name: "Texto 2", ph: "<p:ph type=\"body\" idx=\"1\"/>", x: 838_200, y: 1_825_625, cx: 10_515_600, cy: 4_351_338, anchor: "t", size: 2800, bold: false, align: "l", sample: "Texto" }),
        "</p:spTree></p:cSld><p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>".into(),
        "<p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/><p:sldLayoutId id=\"2147483650\" r:id=\"rId2\"/></p:sldLayoutIdLst>".into(),
        format!("<p:txStyles><p:titleStyle>{}</p:titleStyle><p:bodyStyle>{body_levels}</p:bodyStyle>", level_style("lvl1pPr", 4400, false, 0)),
        format!("<p:otherStyle>{}</p:otherStyle></p:txStyles></p:sldMaster>", level_style("lvl1pPr", 1800, false, 0)),
    ]
    .concat();
    let master_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout2.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/></Relationships>"#;

    let cover_layout = [
        format!("<p:sldLayout {ns} type=\"title\" preserve=\"1\"><p:cSld name=\"Portada\"><p:spTree>{GROUP}"),
        shape(Frame { id: 2, name: "Título 1", ph: "<p:ph type=\"ctrTitle\"/>", x: 1_524_000, y: 1_122_363, cx: 9_144_000, cy: 2_387_600, anchor: "b", size: 6000, bold: true, align: "ctr", sample: "Título" }),
        shape(Frame { id: 3, name: "Subtítulo 2", ph: "<p:ph type=\"subTitle\" idx=\"1\"/>", x: 1_524_000, y: 3_602_038, cx: 9_144_000, cy: 1_655_762, anchor: "t", size: 2400, bold: false, align: "ctr", sample: "Subtítulo" }),
        "</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>".into(),
    ]
    .concat();
    let content_layout = [
        format!("<p:sldLayout {ns} type=\"obj\" preserve=\"1\"><p:cSld name=\"Título y contenido\"><p:spTree>{GROUP}"),
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Título 1"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="es-PE"/><a:t>Título</a:t></a:r></a:p></p:txBody></p:sp>"#.into(),
        r#"<p:sp><p:nvSpPr><p:cNvPr id="3" name="Contenido 2"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="es-PE"/><a:t>Texto</a:t></a:r></a:p></p:txBody></p:sp>"#.into(),
        "</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>".into(),
    ]
    .concat();
    let layout_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/></Relationships>"#;

    let fills = "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>".repeat(3);
    let theme = [
        format!("<a:theme xmlns:a=\"{A}\" name=\"Buddy\"><a:themeElements><a:clrScheme name=\"Buddy\">"),
        r#"<a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="1F2937"/></a:dk2><a:lt2><a:srgbClr val="F3F4F6"/></a:lt2>"#.into(),
        r#"<a:accent1><a:srgbClr val="2F5597"/></a:accent1><a:accent2><a:srgbClr val="ED7D31"/></a:accent2><a:accent3><a:srgbClr val="70AD47"/></a:accent3><a:accent4><a:srgbClr val="FFC000"/></a:accent4><a:accent5><a:srgbClr val="5B9BD5"/></a:accent5><a:accent6><a:srgbClr val="7F7F7F"/></a:accent6><a:hlink><a:srgbClr val="0563C1"/></a:hlink><a:folHlink><a:srgbClr val="954F72"/></a:folHlink>"#.into(),
        r#"</a:clrScheme><a:fontScheme name="Buddy"><a:majorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme>"#.into(),
        format!("<a:fmtScheme name=\"Buddy\"><a:fillStyleLst>{fills}</a:fillStyleLst>"),
        format!("<a:lnStyleLst>{}</a:lnStyleLst>", r#"<a:ln w="9525"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>"#.repeat(3)),
        format!("<a:effectStyleLst>{}</a:effectStyleLst>", "<a:effectStyle><a:effectLst/></a:effectStyle>".repeat(3)),
        format!("<a:bgFillStyleLst>{fills}</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"),
    ]
    .concat();

    let mut out = vec![
        xml::part("[Content_Types].xml", &types),
        xml::part("_rels/.rels", rels),
        xml::part("ppt/presentation.xml", &presentation),
        xml::part("ppt/_rels/presentation.xml.rels", &presentation_rels),
        xml::part("ppt/slideMasters/slideMaster1.xml", &master),
        xml::part("ppt/slideMasters/_rels/slideMaster1.xml.rels", master_rels),
        xml::part("ppt/slideLayouts/slideLayout1.xml", &cover_layout),
        xml::part("ppt/slideLayouts/slideLayout2.xml", &content_layout),
        xml::part("ppt/slideLayouts/_rels/slideLayout1.xml.rels", layout_rels),
        xml::part("ppt/slideLayouts/_rels/slideLayout2.xml.rels", layout_rels),
        xml::part("ppt/theme/theme1.xml", &theme),
    ];
    for (i, slide) in slides.iter().enumerate() {
        let cover = i == 0 && slide.bullets.is_empty();
        out.push(xml::part(&format!("ppt/slides/slide{}.xml", i + 1), &slide_xml(slide, cover)));
        let layout = if cover { 1 } else { 2 };
        out.push(xml::part(
            &format!("ppt/slides/_rels/slide{}.xml.rels", i + 1),
            &format!(
                "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout{layout}.xml\"/></Relationships>"
            ),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_presentation_has_a_cover_and_content_slides() {
        let slides = vec![
            SlideData { title: "Plan & metas".into(), subtitle: Some("2026".into()), bullets: vec![] },
            SlideData { title: "Metas".into(), subtitle: None, bullets: vec!["a < b".into(), "**b**".into()] },
        ];
        let parts = parts(&slides);
        let names: Vec<&str> = parts.iter().map(|(p, _)| p.as_str()).collect();
        for name in [
            "[Content_Types].xml",
            "ppt/presentation.xml",
            "ppt/slideMasters/slideMaster1.xml",
            "ppt/slideLayouts/slideLayout1.xml",
            "ppt/slideLayouts/slideLayout2.xml",
            "ppt/theme/theme1.xml",
            "ppt/slides/slide1.xml",
            "ppt/slides/slide2.xml",
            "ppt/slides/_rels/slide2.xml.rels",
        ] {
            assert!(names.contains(&name), "{name}");
        }
        let text = |path: &str| String::from_utf8(parts.iter().find(|(p, _)| p == path).unwrap().1.clone()).unwrap();
        assert!(text("ppt/slides/_rels/slide1.xml.rels").contains("slideLayout1.xml"), "the cover uses the cover layout");
        assert!(text("ppt/slides/_rels/slide2.xml.rels").contains("slideLayout2.xml"));
        assert!(text("ppt/slides/slide1.xml").contains("Plan &amp; metas") && text("ppt/slides/slide1.xml").contains("subTitle"));
        assert!(text("ppt/slides/slide2.xml").contains("b=\"1\""));
        assert!(text("ppt/slides/slide2.xml").contains("a &lt; b"));
        assert!(text("ppt/presentation.xml").contains("<p:sldId id=\"257\" r:id=\"rId3\"/>"));
    }

    #[test]
    fn a_first_slide_with_bullets_is_not_a_cover() {
        let parts = parts(&[SlideData { title: "Uno".into(), subtitle: Some("x".into()), bullets: vec!["a".into()] }]);
        let rels = parts.iter().find(|(p, _)| p == "ppt/slides/_rels/slide1.xml.rels").unwrap();
        assert!(String::from_utf8_lossy(&rels.1).contains("slideLayout2.xml"));
    }
}
