// Ported from MIKA (MIT, revision d050bc5): apps/macos/Sources/Providers/OfficeFiles.swift (OfficeXML)
//! The little XML kit the three writers share: escaping, the part header, A1 column names, number text and the
//! `**bold**` / `*italic*` / `` `code` `` runs inside a line of text.

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
}

impl Run {
    fn plain(text: &str, bold: bool, italic: bool) -> Self {
        Run { text: text.to_string(), bold, italic, code: false }
    }
}

/// `**bold**`, `*italic*` and `` `code` `` in a line of text, as runs. A lone `*` followed by a space is just text.
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
        let r = |text: &str, bold, italic, code| Run { text: text.into(), bold, italic, code };
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
}
