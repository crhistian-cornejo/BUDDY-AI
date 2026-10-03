// Ported from MIKA (MIT, revision d050bc5): apps/macos/Sources/Providers/OfficeFiles.swift (OfficeXML)
//! The little XML kit the three writers share: escaping, the part header, A1 column names, number text, the
//! `**bold**` / `*italic*` / `` `code` `` / `[link](https://…)` runs inside a line of text, the document properties
//! and dates.

/// The declaration every part starts with.
pub const HEADER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

/// One file inside the archive: its path (with `/`) and its bytes.
pub type Part = (String, Vec<u8>);

/// Text safe for an XML node or a double-quoted attribute: the special characters escaped and the characters XML
/// 1.0 cannot carry (controls other than tab and newlines, U+FFFE, U+FFFF) dropped.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out
}

/// A part: the XML declaration plus `xml`, as bytes.
pub fn part(path: &str, xml: &str) -> Part {
    let mut data = String::with_capacity(HEADER.len() + xml.len());
    data.push_str(HEADER);
    data.push_str(xml);
    (path.to_string(), data.into_bytes())
}

/// A1-style column name: 0 → "A", 26 → "AA".
pub fn column(index: usize) -> String {
    let mut n = index + 1;
    let mut out = Vec::new();
    while n > 0 {
        let r = (n - 1) % 26;
        out.push(b'A' + r as u8);
        n = (n - 1) / 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// A number as a cell or table shows it: whole numbers without a decimal point, the rest as short as they go.
pub fn number_text(n: f64) -> String {
    if n == n.round() && n.abs() < 1e15 {
        (n as i64).to_string()
    } else if n.abs() >= 1e15 || n.abs() < 1e-5 {
        // Display never uses an exponent, which would spell 1e300 with 300 zeros.
        format!("{n:e}")
    } else {
        n.to_string()
    }
}

/// The first `max` characters of `text`.
pub fn prefix(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// A stretch of text with one look.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    /// The web or mail address the text points to (`[text](https://…)`).
    pub link: Option<String>,
}

impl Run {
    fn plain(text: &str, bold: bool, italic: bool) -> Self {
        Run { text: text.to_string(), bold, italic, ..Run::default() }
    }
}

/// Longest address a link may carry.
const MAX_LINK: usize = 2000;

/// An address a document may link to: http(s) or mail only, no spaces, no quotes or angle brackets.
pub fn safe_link(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let scheme = ["https://", "http://", "mailto:"].iter().any(|s| lower.starts_with(s) && lower.len() > s.len());
    scheme && url.len() <= MAX_LINK && !url.chars().any(|c| c.is_whitespace() || c.is_control() || "\"<>`".contains(c))
}

/// `[text](url)` starting at `chars[i]`: the text, the address and the index just past it.
fn link_at(chars: &[char], i: usize) -> Option<(String, String, usize)> {
    let close = i + 1 + chars[i + 1..].iter().position(|&c| c == ']' || c == '[')?;
    if chars[close] != ']' || close == i + 1 || chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = close + 2 + chars[close + 2..].iter().position(|&c| c == ')')?;
    let url: String = chars[close + 2..end].iter().collect();
    safe_link(&url).then(|| (chars[i + 1..close].iter().collect(), url, end + 1))
}

/// `**bold**`, `*italic*`, `` `code` `` and `[text](https://…)` in a line of text, as runs. A lone `*` followed by a
/// space is just text, and so is a link to anything but http(s) or mail.
pub fn runs(source: &str) -> Vec<Run> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut buffer = String::new();
    let (mut bold, mut italic) = (false, false);
    let flush = |out: &mut Vec<Run>, buffer: &mut String, bold: bool, italic: bool| {
        if !buffer.is_empty() {
            out.push(Run::plain(buffer, bold, italic));
            buffer.clear();
        }
    };
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`'
            && let Some(offset) = chars[i + 1..].iter().position(|&c| c == '`')
            && offset > 0
        {
            let end = i + 1 + offset;
            flush(&mut out, &mut buffer, bold, italic);
            out.push(Run { text: chars[i + 1..end].iter().collect(), code: true, ..Run::default() });
            i = end + 1;
            continue;
        }
        if chars[i] == '['
            && let Some((text, url, next)) = link_at(&chars, i)
        {
            flush(&mut out, &mut buffer, bold, italic);
            out.push(Run { text, bold, italic, code: false, link: Some(url) });
            i = next;
            continue;
        }
        if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            flush(&mut out, &mut buffer, bold, italic);
            bold = !bold;
            i += 2;
            continue;
        }
        if chars[i] == '*' && i + 1 < chars.len() && (chars[i + 1] != ' ' || italic) {
            flush(&mut out, &mut buffer, bold, italic);
            italic = !italic;
            i += 1;
            continue;
        }
        buffer.push(chars[i]);
        i += 1;
    }
    flush(&mut out, &mut buffer, bold, italic);
    out
}

/// The plain text of a line with marks: what a table of contents or a reader shows.
pub fn plain(source: &str) -> String {
    runs(source).into_iter().map(|r| r.text).collect()
}

// MARK: dates

/// Days from 1970-01-01 to the given civil date (proleptic Gregorian; Howard Hinnant's algorithm).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of a day count from 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { yoe + era * 400 + 1 } else { yoe + era * 400 }, m, d)
}

/// Now, as the W3C date-time Office keeps in its document properties (UTC).
pub fn now_w3c() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
}

// MARK: document properties

pub const CORE_TYPE: &str = "application/vnd.openxmlformats-package.core-properties+xml";
pub const APP_TYPE: &str = "application/vnd.openxmlformats-officedocument.extended-properties+xml";
/// The two `_rels/.rels` entries that point at the properties.
pub const PROPS_RELS: &str = r#"<Relationship Id="rIdCore" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/><Relationship Id="rIdApp" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>"#;

/// `docProps/core.xml` and `docProps/app.xml`: title, subject, author and dates, which Word, Excel and PowerPoint show
/// in the file's properties (and the Finder / Explorer in its details).
pub fn properties(title: Option<&str>, subject: Option<&str>, author: Option<&str>) -> Vec<Part> {
    let now = now_w3c();
    let mut core = String::from(
        r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">"#,
    );
    for (tag, value) in [("dc:title", title), ("dc:subject", subject), ("dc:creator", author)] {
        if let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) {
            core.push_str(&format!("<{tag}>{}</{tag}>", escape(&prefix(value, 500))));
        }
    }
    core.push_str(&format!(
        "<cp:lastModifiedBy>Buddy</cp:lastModifiedBy><dcterms:created xsi:type=\"dcterms:W3CDTF\">{now}</dcterms:created><dcterms:modified xsi:type=\"dcterms:W3CDTF\">{now}</dcterms:modified></cp:coreProperties>"
    ));
    let app = r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>Buddy</Application></Properties>"#;
    vec![part("docProps/core.xml", &core), part("docProps/app.xml", app)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_escaped_and_control_characters_dropped() {
        assert_eq!(escape("a & b < c > \"d\""), "a &amp; b &lt; c &gt; &quot;d&quot;");
        assert_eq!(escape("a\u{0}b\u{8}c\td\n\u{FFFE}"), "abc\td\n");
        assert_eq!(escape("ñandú ✓"), "ñandú ✓");
    }

    #[test]
    fn columns_are_named_like_excel() {
        assert_eq!(column(0), "A");
        assert_eq!(column(25), "Z");
        assert_eq!(column(26), "AA");
        assert_eq!(column(701), "ZZ");
        assert_eq!(column(702), "AAA");
    }

    #[test]
    fn numbers_read_naturally() {
        assert_eq!(number_text(3.0), "3");
        assert_eq!(number_text(-0.0), "0");
        assert_eq!(number_text(1.5), "1.5");
        assert_eq!(number_text(1e20), "1e20");
        assert_eq!(number_text(0.000001), "1e-6");
    }

    #[test]
    fn markdown_marks_become_runs() {
        let r = |text: &str, bold, italic, code| Run { text: text.into(), bold, italic, code, link: None };
        assert_eq!(
            runs("a **b** *c* `d` e"),
            vec![
                r("a ", false, false, false),
                r("b", true, false, false),
                r(" ", false, false, false),
                r("c", false, true, false),
                r(" ", false, false, false),
                r("d", false, false, true),
                r(" e", false, false, false),
            ]
        );
        let joined: String = runs("2 * 3 = 6").into_iter().map(|r| r.text).collect();
        assert_eq!(joined, "2 * 3 = 6", "a lone asterisk is just text");
        let joined: String = runs("a `` b *").into_iter().map(|r| r.text).collect();
        assert_eq!(joined, "a `` b *", "an empty code span and a trailing asterisk stay as typed");
        assert!(runs("").is_empty());
    }

    #[test]
    fn links_become_runs_only_to_the_web_or_mail() {
        let runs = runs("ver [la guía](https://ejemplo.com/a?b=1) y **[x](mailto:a@b.c)**");
        assert_eq!(runs[1], Run { text: "la guía".into(), link: Some("https://ejemplo.com/a?b=1".into()), ..Run::default() });
        assert_eq!(runs[3], Run { text: "x".into(), bold: true, link: Some("mailto:a@b.c".into()), ..Run::default() });
        for text in ["[a](javascript:alert(1))", "[a](file:///etc/passwd)", "[](https://x.com)", "[a] (https://x.com)", "[a](https://x .com)"] {
            assert!(super::runs(text).iter().all(|r| r.link.is_none()), "{text}");
            assert_eq!(plain(text), text, "left as typed");
        }
    }

    #[test]
    fn dates_go_both_ways() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 10, 2), 20_728);
        assert_eq!(civil_from_days(20_728), (2026, 10, 2));
        assert_eq!(civil_from_days(days_from_civil(1899, 12, 30)), (1899, 12, 30));
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
        let now = now_w3c();
        assert_eq!(now.len(), 20);
        assert!(now.ends_with('Z'));
    }
}
