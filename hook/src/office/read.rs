//! `read_document`: the text of a file the user shared (or Buddy made), for the agent to work with. Word gives its
//! paragraphs (headings marked with `#`, list entries with `-`, tables as tab-separated rows), Excel its sheets as
//! tab-separated rows, PowerPoint its slides with their notes, a PDF its text layer, and a text file itself. The
//! result is capped; what is read is data for the agent, never instructions.

use std::collections::HashMap;

use super::access::Access;
use super::zip::Archive;
use super::{OfficeError, pdf};

/// Longest text returned.
pub const MAX_CHARS: usize = 200_000;
/// Largest file read.
const MAX_FILE: u64 = 100 * 1024 * 1024;
/// Largest part of an Office file once unpacked.
const MAX_PART: usize = 64 * 1024 * 1024;

pub fn read_document(raw: &str, access: &Access) -> Result<String, OfficeError> {
    let (path, data) = access.read(raw, MAX_FILE)?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let text = match ext.as_str() {
        "docx" | "docm" | "dotx" => docx_text(&Archive::open(&data)?)?,
        "xlsx" | "xlsm" | "xltx" => xlsx_text(&Archive::open(&data)?)?,
        "pptx" | "pptm" | "potx" => pptx_text(&Archive::open(&data)?)?,
        "pdf" => tidy(&pdf::text(&data, MAX_CHARS + 1)?),
        "txt" | "md" | "markdown" | "csv" | "tsv" | "json" | "jsonl" | "log" | "yaml" | "yml" | "xml" | "html" | "htm" | "rtf" | "ini" | "toml" => plain_text(&data),
        "doc" | "xls" | "ppt" => {
            return Err(OfficeError(format!(
                "El formato .{ext} (Office 97-2003) no lo sé leer. Pide al usuario que lo guarde como .{ext}x o PDF."
            )));
        }
        _ => {
            return Err(OfficeError(format!(
                "No sé leer archivos .{ext}. Leo Word (.docx), Excel (.xlsx), PowerPoint (.pptx), PDF y texto (.txt, .md, .csv, .json…)."
            )));
        }
    };
    let text = text.trim();
    if text.is_empty() {
        return Ok(format!("{}\n\n(El archivo no tiene texto.)", path.display()));
    }
    Ok(format!("{}\n\n{}", path.display(), cap(text, MAX_CHARS)))
}

/// The first `max` characters of `text`, with a note when something was cut.
pub fn cap(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        None => text.to_string(),
        Some((at, _)) => {
            let total = text.chars().count();
            format!(
                "{}\n\n[Recortado: se muestran los primeros {max} caracteres de {total}. El resto del documento no se incluye.]",
                &text[..at]
            )
        }
    }
}

/// UTF-8 (without its BOM), or Windows-1252 when the bytes are not UTF-8 (old Excel CSVs).
fn plain_text(data: &[u8]) -> String {
    let data = data.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(data);
    match std::str::from_utf8(data) {
        Ok(s) => s.to_string(),
        Err(_) => data.iter().map(|&b| cp1252(b)).collect(),
    }
}

/// One byte of Windows-1252 (or Latin-1, which it extends).
pub fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}', '\u{90}', '‘', '’', '“',
        '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
    ];
    if (0x80..0xA0).contains(&b) { HIGH[(b - 0x80) as usize] } else { b as char }
}

// MARK: XML

/// One step through an XML text.
#[derive(Debug, PartialEq)]
pub enum Event<'a> {
    /// `<name attrs>` or `<name attrs/>`: the local name (no prefix) and the raw attributes.
    Start { name: &'a str, attrs: &'a str, empty: bool },
    End(&'a str),
    /// Text between tags, still escaped (see `unescape`), or a CDATA section as is.
    Text(&'a str),
    CData(&'a str),
}

/// A forgiving pull parser: enough for the XML inside Office files. Comments, declarations and processing
/// instructions are skipped.
pub struct Scanner<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Scanner<'a> {
    pub fn new(s: &'a str) -> Self {
        Scanner { s, i: 0 }
    }
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

impl<'a> Iterator for Scanner<'a> {
    type Item = Event<'a>;

    fn next(&mut self) -> Option<Event<'a>> {
        loop {
            let rest = &self.s[self.i..];
            if rest.is_empty() {
                return None;
            }
            if !rest.starts_with('<') {
                let end = rest.find('<').unwrap_or(rest.len());
                self.i += end;
                return Some(Event::Text(&rest[..end]));
            }
            let skip_to = |pat: &str, from: usize| rest[from..].find(pat).map(|p| from + p + pat.len()).unwrap_or(rest.len());
            if rest.starts_with("<!--") {
                self.i += skip_to("-->", 4);
                continue;
            }
            if let Some(body) = rest.strip_prefix("<![CDATA[") {
                let end = body.find("]]>").unwrap_or(body.len());
                self.i += 9 + (end + 3).min(body.len());
                return Some(Event::CData(&body[..end]));
            }
            if rest.starts_with("<?") {
                self.i += skip_to("?>", 2);
                continue;
            }
            if rest.starts_with("<!") {
                self.i += skip_to(">", 2);
                continue;
            }
            // A tag: find its end, minding quoted attribute values.
            let bytes = rest.as_bytes();
            let mut quote = 0u8;
            let mut end = None;
            for (k, &b) in bytes.iter().enumerate().skip(1) {
                if quote != 0 {
                    if b == quote {
                        quote = 0;
                    }
                } else if b == b'"' || b == b'\'' {
                    quote = b;
                } else if b == b'>' {
                    end = Some(k);
                    break;
                }
            }
            let Some(end) = end else {
                self.i = self.s.len();
                return None;
            };
            self.i += end + 1;
            let inner = &rest[1..end];
            if let Some(name) = inner.strip_prefix('/') {
                return Some(Event::End(local(name.trim())));
            }
            let empty = inner.ends_with('/');
            let inner = inner.strip_suffix('/').unwrap_or(inner);
            let split = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
            return Some(Event::Start { name: local(&inner[..split]), attrs: &inner[split..], empty });
        }
    }
}

/// The value of the attribute whose local name is `key`, unescaped.
pub fn attr(attrs: &str, key: &str) -> Option<String> {
    let mut rest = attrs;
    loop {
        rest = rest.trim_start();
        let eq = rest.find('=')?;
        let name = rest[..eq].trim();
        let after = rest[eq + 1..].trim_start();
        let quote = after.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let close = after[1..].find(quote)? + 1;
        if local(name) == key {
            return Some(unescape(&after[1..close]));
        }
        rest = &after[close + 1..];
    }
}

/// The relationship id of an element (`r:id`, whatever the prefix): the prefixed attribute named `id`, so a plain
/// `id` beside it (as in `<p:sldId id="256" r:id="rId2"/>`) is not taken for it.
fn rel_id(attrs: &str) -> Option<String> {
    let mut rest = attrs;
    loop {
        rest = rest.trim_start();
        let eq = rest.find('=')?;
        let name = rest[..eq].trim();
        let after = rest[eq + 1..].trim_start();
        let quote = after.chars().next()?;
        let close = after[1..].find(quote)? + 1;
        if name.contains(':') && local(name) == "id" {
            return Some(unescape(&after[1..close]));
        }
        rest = &after[close + 1..];
    }
}

/// XML text with its entities and character references resolved.
pub fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(semi) = rest.find(';').filter(|&s| s <= 12) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..semi];
        let c = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn part(archive: &Archive, name: &str) -> Result<String, OfficeError> {
    Ok(String::from_utf8_lossy(&archive.read(name, MAX_PART)?).into_owned())
}

/// `Id → target` of a relationships part, each target resolved against the folder of the part it belongs to.
fn relationships(archive: &Archive, owner: &str) -> HashMap<String, String> {
    let (dir, file) = owner.rsplit_once('/').unwrap_or(("", owner));
    let rels_name = if dir.is_empty() { format!("_rels/{file}.rels") } else { format!("{dir}/_rels/{file}.rels") };
    let Ok(xml) = part(archive, &rels_name) else { return HashMap::new() };
    let mut out = HashMap::new();
    for event in Scanner::new(&xml) {
        if let Event::Start { name: "Relationship", attrs, .. } = event
            && let (Some(id), Some(target)) = (attr(attrs, "Id"), attr(attrs, "Target"))
        {
            if attr(attrs, "TargetMode").as_deref() == Some("External") {
                continue;
            }
            out.insert(id, resolve(dir, &target));
        }
    }
    out
}

/// `target` relative to the folder `dir` inside the package (or absolute from its root), with `..` resolved.
fn resolve(dir: &str, target: &str) -> String {
    let mut parts: Vec<&str> = if target.starts_with('/') { Vec::new() } else { dir.split('/').filter(|p| !p.is_empty()).collect() };
    for piece in target.split('/') {
        match piece {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

// MARK: Word

/// A heading level from a paragraph style id, in English or Spanish Word (`Heading2`, `Ttulo2`, `Title`).
fn heading_level(style: &str) -> Option<usize> {
    let lower = style.to_lowercase();
    let digits: String = lower.chars().filter(|c| c.is_ascii_digit()).collect();
    let named = lower.starts_with("heading") || lower.starts_with("ttulo") || lower.starts_with("titulo") || lower.starts_with("título");
    if named && !digits.is_empty() {
        return digits.parse::<usize>().ok().map(|n| n.clamp(1, 6));
    }
    (lower == "title" || lower == "ttulo" || lower == "titulo" || lower == "título").then_some(1)
}

fn docx_text(archive: &Archive) -> Result<String, OfficeError> {
    let main = if archive.contains("word/document.xml") {
        "word/document.xml".to_string()
    } else {
        archive.names().find(|n| n.starts_with("word/document") && n.ends_with(".xml")).map(str::to_string).ok_or_else(|| {
            OfficeError::new("No encuentro el texto del documento (word/document.xml).")
        })?
    };
    Ok(word_body(&part(archive, &main)?))
}

/// The text of a WordprocessingML body.
fn word_body(xml: &str) -> String {
    let mut out = String::new();
    let mut para = String::new();
    let mut prefix = String::new();
    let mut in_text = false;
    let mut cells: Vec<Vec<String>> = Vec::new(); // one per open table: the cells of the current row
    let mut cell: Vec<String> = Vec::new(); // one per open table: the current cell's text
    let mut in_ppr = false;
    for event in Scanner::new(xml) {
        match event {
            Event::Start { name, attrs, empty } => match name {
                "p" if !empty => {
                    para.clear();
                    prefix.clear();
                }
                "pPr" if !empty => in_ppr = true,
                "pStyle" if in_ppr => {
                    if let Some(level) = attr(attrs, "val").as_deref().and_then(heading_level) {
                        prefix = format!("{} ", "#".repeat(level));
                    }
                }
                "ilvl" if in_ppr && prefix.is_empty() => {
                    let level: usize = attr(attrs, "val").and_then(|v| v.parse().ok()).unwrap_or(0);
                    prefix = format!("{}- ", "  ".repeat(level.min(8)));
                }
                "t" if !empty => in_text = true,
                "tab" if !in_ppr => para.push('\t'),
                "br" | "cr" => para.push('\n'),
                "noBreakHyphen" => para.push('-'),
                "tbl" if !empty => {
                    cells.push(Vec::new());
                    cell.push(String::new());
                    if !out.is_empty() && !out.ends_with("\n\n") {
                        out.push('\n');
                    }
                }
                "tr" if !empty => {
                    if let Some(row) = cells.last_mut() {
                        row.clear();
                    }
                }
                "tc" if !empty => {
                    if let Some(c) = cell.last_mut() {
                        c.clear();
                    }
                }
                _ => {}
            },
            Event::End(name) => match name {
                "t" => in_text = false,
                "pPr" => in_ppr = false,
                "p" => {
                    let line = format!("{prefix}{}", para.trim_end());
                    match cell.last_mut() {
                        Some(c) => {
                            if !c.is_empty() && !line.trim().is_empty() {
                                c.push(' ');
                            }
                            c.push_str(line.trim());
                        }
                        None => {
                            out.push_str(&line);
                            out.push('\n');
                        }
                    }
                    para.clear();
                    prefix.clear();
                }
                "tc" => {
                    let text = cell.last().cloned().unwrap_or_default().replace(['\t', '\n'], " ");
                    if let Some(row) = cells.last_mut() {
                        row.push(text);
                    }
                }
                "tr" => {
                    let row = cells.last().map(|r| r.join("\t")).unwrap_or_default();
                    if cells.len() > 1 {
                        // A table inside a cell: its rows join the outer cell's text.
                        if let Some(outer) = cell.iter_mut().rev().nth(1) {
                            outer.push(' ');
                            outer.push_str(&row.replace('\t', " | "));
                        }
                    } else {
                        out.push_str(&row);
                        out.push('\n');
                    }
                }
                "tbl" => {
                    cells.pop();
                    cell.pop();
                    if cells.is_empty() {
                        out.push('\n');
                    }
                }
                _ => {}
            },
            Event::Text(t) if in_text => para.push_str(&unescape(t)),
            Event::CData(t) if in_text => para.push_str(t),
            _ => {}
        }
    }
    tidy(&out)
}

/// At most one empty line in a row, no trailing spaces.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

// MARK: Excel

/// Built-in number formats that are dates or times.
fn builtin_date(id: u32) -> bool {
    (14..=22).contains(&id) || (45..=47).contains(&id) || (27..=36).contains(&id) || (50..=58).contains(&id)
}

fn date_code(code: &str) -> bool {
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

/// For every cell style index: does it show a date?
fn date_styles(archive: &Archive) -> Vec<bool> {
    let Ok(xml) = part(archive, "xl/styles.xml") else { return Vec::new() };
    let mut custom: HashMap<u32, bool> = HashMap::new();
    let mut out = Vec::new();
    let mut in_xfs = false;
    for event in Scanner::new(&xml) {
        match event {
            Event::Start { name: "numFmt", attrs, .. } => {
                if let (Some(id), Some(code)) = (attr(attrs, "numFmtId").and_then(|i| i.parse().ok()), attr(attrs, "formatCode")) {
                    custom.insert(id, date_code(&code));
                }
            }
            Event::Start { name: "cellXfs", empty: false, .. } => in_xfs = true,
            Event::End("cellXfs") => in_xfs = false,
            Event::Start { name: "xf", attrs, .. } if in_xfs => {
                let id: u32 = attr(attrs, "numFmtId").and_then(|i| i.parse().ok()).unwrap_or(0);
                out.push(custom.get(&id).copied().unwrap_or_else(|| builtin_date(id)));
            }
            _ => {}
        }
    }
    out
}

/// An Excel serial date as `2026-10-02` (and `14:30` when it has a time).
fn serial_date(serial: f64) -> Option<String> {
    if !(1.0..2_958_466.0).contains(&serial) {
        return None;
    }
    let days = serial.floor() as i64;
    let (y, m, d) = super::xml::civil_from_days(days + super::xml::days_from_civil(1899, 12, 30));
    let secs = ((serial - serial.floor()) * 86_400.0).round() as i64;
    Some(if secs == 0 || secs >= 86_400 {
        format!("{y:04}-{m:02}-{d:02}")
    } else {
        format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", secs / 3600, secs / 60 % 60)
    })
}

/// "B12" → (1, 11).
fn cell_ref(r: &str) -> Option<(usize, usize)> {
    let letters: String = r.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    if letters.is_empty() {
        return None;
    }
    let col = letters.chars().fold(0usize, |acc, c| acc * 26 + (c.to_ascii_uppercase() as usize - 'A' as usize + 1)) - 1;
    let row: usize = r[letters.len()..].parse().ok()?;
    Some((col, row.checked_sub(1)?))
}

fn shared_strings(archive: &Archive) -> Vec<String> {
    let Ok(xml) = part(archive, "xl/sharedStrings.xml") else { return Vec::new() };
    let mut out = Vec::new();
    let mut current = String::new();
    let (mut in_t, mut in_phonetic) = (false, false);
    for event in Scanner::new(&xml) {
        match event {
            Event::Start { name: "si", .. } => current.clear(),
            Event::End("si") => out.push(std::mem::take(&mut current)),
            Event::Start { name: "rPh", empty: false, .. } => in_phonetic = true,
            Event::End("rPh") => in_phonetic = false,
            Event::Start { name: "t", empty: false, .. } => in_t = true,
            Event::End("t") => in_t = false,
            Event::Text(t) if in_t && !in_phonetic => current.push_str(&unescape(t)),
            _ => {}
        }
    }
    out
}

fn xlsx_text(archive: &Archive) -> Result<String, OfficeError> {
    let workbook = part(archive, "xl/workbook.xml")?;
    let rels = relationships(archive, "xl/workbook.xml");
    let shared = shared_strings(archive);
    let dates = date_styles(archive);
    let mut out = String::new();
    for event in Scanner::new(&workbook) {
        let Event::Start { name: "sheet", attrs, .. } = event else { continue };
        let name = attr(attrs, "name").unwrap_or_default();
        let hidden = attr(attrs, "state").is_some_and(|s| s != "visible");
        let Some(target) = rel_id(attrs).and_then(|id| rels.get(&id).cloned()) else { continue };
        out.push_str(&format!("## Hoja: {name}{}\n", if hidden { " (oculta)" } else { "" }));
        match part(archive, &target) {
            Ok(xml) => out.push_str(&sheet_text(&xml, &shared, &dates)),
            Err(_) => out.push_str("(No pude leer esta hoja.)\n"),
        }
        out.push('\n');
        if out.len() > MAX_CHARS * 4 {
            break;
        }
    }
    Ok(out)
}

fn sheet_text(xml: &str, shared: &[String], dates: &[bool]) -> String {
    let mut out = String::new();
    let mut row: Vec<(usize, String)> = Vec::new();
    let (mut kind, mut style, mut col) = (String::new(), 0usize, 0usize);
    let (mut value, mut formula, mut inline) = (String::new(), String::new(), String::new());
    let mut field = "";
    let mut next_col = 0usize;
    let flush_row = |out: &mut String, row: &mut Vec<(usize, String)>| {
        if row.iter().all(|(_, v)| v.is_empty()) {
            row.clear();
            return;
        }
        let width = row.iter().map(|(c, _)| c + 1).max().unwrap_or(0).min(16_384);
        let mut cells = vec![String::new(); width];
        for (c, v) in row.drain(..) {
            if c < width {
                cells[c] = v.replace(['\t', '\n', '\r'], " ");
            }
        }
        out.push_str(cells.join("\t").trim_end_matches('\t'));
        out.push('\n');
    };
    for event in Scanner::new(xml) {
        match event {
            Event::Start { name: "row", .. } => {
                row.clear();
                next_col = 0;
            }
            Event::End("row") => flush_row(&mut out, &mut row),
            Event::Start { name: "c", attrs, empty } => {
                kind = attr(attrs, "t").unwrap_or_default();
                style = attr(attrs, "s").and_then(|s| s.parse().ok()).unwrap_or(0);
                col = attr(attrs, "r").as_deref().and_then(cell_ref).map(|(c, _)| c).unwrap_or(next_col);
                next_col = col + 1;
                value.clear();
                formula.clear();
                inline.clear();
                if empty {
                    row.push((col, String::new()));
                }
            }
            Event::Start { name: "v", empty: false, .. } => field = "v",
            Event::Start { name: "f", empty: false, .. } => field = "f",
            Event::Start { name: "t", empty: false, .. } => field = "t",
            Event::End("v" | "f" | "t") => field = "",
            Event::Text(t) => match field {
                "v" => value.push_str(&unescape(t)),
                "f" => formula.push_str(&unescape(t)),
                "t" => inline.push_str(&unescape(t)),
                _ => {}
            },
            Event::End("c") => {
                let shown = match kind.as_str() {
                    "s" => value.trim().parse::<usize>().ok().and_then(|i| shared.get(i).cloned()).unwrap_or_default(),
                    "inlineStr" => inline.clone(),
                    "b" => if value.trim() == "1" { "VERDADERO" } else { "FALSO" }.to_string(),
                    "str" | "e" => value.clone(),
                    _ if value.is_empty() && !formula.is_empty() => format!("={formula}"),
                    _ => {
                        let date = dates.get(style).copied().unwrap_or(false);
                        match value.trim().parse::<f64>() {
                            Ok(n) if date => serial_date(n).unwrap_or_else(|| value.clone()),
                            _ => value.clone(),
                        }
                    }
                };
                let shown = if kind.is_empty() && value.is_empty() && formula.is_empty() { String::new() } else { shown };
                row.push((col, shown));
            }
            _ => {}
        }
    }
    flush_row(&mut out, &mut row);
    if out.is_empty() { "(Hoja vacía)\n".into() } else { out }
}

// MARK: PowerPoint

/// The text of one slide (or notes page): one line per paragraph, shapes in order, slide numbers, dates and
/// footers left out.
fn drawing_text(xml: &str) -> String {
    let mut out = String::new();
    let mut shape = String::new();
    let mut para = String::new();
    let (mut in_t, mut skip, mut depth) = (false, false, 0usize);
    for event in Scanner::new(xml) {
        match event {
            Event::Start { name: "sp" | "graphicFrame", empty: false, .. } => {
                depth += 1;
                if depth == 1 {
                    shape.clear();
                    skip = false;
                }
            }
            Event::End("sp" | "graphicFrame") => {
                depth = depth.saturating_sub(1);
                if depth == 0 && !skip && !shape.trim().is_empty() {
                    out.push_str(shape.trim_end());
                    out.push('\n');
                }
            }
            Event::Start { name: "ph", attrs, .. } => {
                if matches!(attr(attrs, "type").as_deref(), Some("sldNum" | "dt" | "ftr" | "hdr" | "sldImg")) {
                    skip = true;
                }
            }
            Event::Start { name: "t", empty: false, .. } => in_t = true,
            Event::End("t") => in_t = false,
            Event::Start { name: "br", .. } => para.push('\n'),
            Event::End("p") => {
                let line = para.trim();
                if !line.is_empty() {
                    let target = if depth > 0 { &mut shape } else { &mut out };
                    target.push_str(line);
                    target.push('\n');
                }
                para.clear();
            }
            Event::End("tc") => para.push('\t'),
            Event::Text(t) if in_t => para.push_str(&unescape(t)),
            _ => {}
        }
    }
    out
}

fn pptx_text(archive: &Archive) -> Result<String, OfficeError> {
    let presentation = part(archive, "ppt/presentation.xml")?;
    let rels = relationships(archive, "ppt/presentation.xml");
    let mut slides: Vec<String> = Vec::new();
    for event in Scanner::new(&presentation) {
        if let Event::Start { name: "sldId", attrs, .. } = event
            && let Some(target) = rel_id(attrs).and_then(|id| rels.get(&id).cloned())
        {
            slides.push(target);
        }
    }
    let mut out = String::new();
    for (i, slide) in slides.iter().enumerate() {
        out.push_str(&format!("## Diapositiva {}\n", i + 1));
        match part(archive, slide) {
            Ok(xml) => out.push_str(&drawing_text(&xml)),
            Err(_) => out.push_str("(No pude leer esta diapositiva.)\n"),
        }
        let slide_rels = relationships(archive, slide);
        let notes = slide_rels.values().find(|t| t.contains("notesSlides/"));
        if let Some(notes) = notes
            && let Ok(xml) = part(archive, notes)
        {
            let text = drawing_text(&xml);
            if !text.trim().is_empty() {
                out.push_str(&format!("Notas: {}\n", text.trim().replace('\n', "\n  ")));
            }
        }
        out.push('\n');
        if out.len() > MAX_CHARS * 4 {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office::test_support::TempDir;
    use crate::office::{self, DOCUMENT, PRESENTATION, SPREADSHEET};
    use serde_json::json;

    #[test]
    fn the_scanner_walks_office_xml() {
        let xml = r#"<?xml version="1.0"?><!-- c --><w:p a="1>2" b='x'><w:t xml:space="preserve">A &amp; B &#233;&#x41;</w:t><w:br/><![CDATA[<raw>]]></w:p>"#;
        let events: Vec<Event> = Scanner::new(xml).collect();
        assert_eq!(events[0], Event::Start { name: "p", attrs: r#" a="1>2" b='x'"#, empty: false });
        assert_eq!(attr(r#" a="1>2" b='x'"#, "b").as_deref(), Some("x"));
        assert_eq!(attr(r#" w:val="Heading1""#, "val").as_deref(), Some("Heading1"));
        assert_eq!(rel_id(r#" id="256" r:id="rId2""#).as_deref(), Some("rId2"));
        assert_eq!(events[2], Event::Text("A &amp; B &#233;&#x41;"));
        assert_eq!(unescape("A &amp; B &#233;&#x41; &bogus; & x"), "A & B éA &bogus; & x");
        assert_eq!(events[4], Event::Start { name: "br", attrs: "", empty: true });
        assert_eq!(events[5], Event::CData("<raw>"));
        assert_eq!(events[6], Event::End("p"));
        assert_eq!(resolve("ppt/slides", "../media/a.png"), "ppt/media/a.png");
        assert_eq!(resolve("xl", "/xl/worksheets/sheet1.xml"), "xl/worksheets/sheet1.xml");
        assert_eq!(cell_ref("AB12"), Some((27, 11)));
        assert_eq!(serial_date(46_297.0).as_deref(), Some("2026-10-02"));
        assert_eq!(serial_date(46_297.5).as_deref(), Some("2026-10-02 12:00"));
        assert_eq!(heading_level("Ttulo2"), Some(2));
        assert_eq!(heading_level("Title"), Some(1));
        assert_eq!(heading_level("Normal"), None);
    }

    #[test]
    fn what_buddy_writes_it_reads_back() {
        let tmp = TempDir::new("read");
        let access = Access::new(&tmp.0, []);
        let doc = office::run(
            DOCUMENT,
            &json!({ "file": "d", "title": "Informe anual", "toc": true, "blocks": [
                { "type": "heading", "level": 1, "text": "Resumen" },
                { "type": "paragraph", "text": "Ventas **crecieron** [ver](https://buddy.app)." },
                { "type": "bullets", "items": ["uno", { "text": "dos", "items": ["dos.a"] }] },
                { "type": "table", "header": ["Mes", "Monto"], "rows": [["Enero", "100"], ["Febrero", "200"]] }
            ] }),
            &tmp.0,
            &access,
        )
        .unwrap();
        let text = read_document(&doc.to_string_lossy(), &access).unwrap();
        for needle in ["# Informe anual", "# Resumen", "Ventas crecieron ver.", "- uno", "  - dos.a", "Mes\tMonto", "Febrero\t200"] {
            assert!(text.contains(needle), "{needle}\n{text}");
        }

        let book = office::run(
            SPREADSHEET,
            &json!({ "file": "b", "sheets": [
                { "name": "Ventas", "columns": [{ "format": "date" }, { "format": "currency" }], "rows": [["Fecha", "Monto"], ["2026-10-02", 10.5], ["", "=SUM(B2:B2)"]] },
                { "name": "Otra", "rows": [[true, null, "x"]] }
            ] }),
            &tmp.0,
            &access,
        )
        .unwrap();
        let text = read_document("b.xlsx", &access).unwrap();
        for needle in ["## Hoja: Ventas", "Fecha\tMonto", "2026-10-02\t10.5", "\t=SUM(B2:B2)", "## Hoja: Otra", "VERDADERO\t\tx"] {
            assert!(text.contains(needle), "{needle}\n{text}");
        }
        assert!(book.is_file());

        office::run(
            PRESENTATION,
            &json!({ "file": "p", "slides": [
                { "title": "Plan 2027", "subtitle": "Equipo" },
                { "title": "Metas", "bullets": ["Crecer", { "text": "Contratar", "items": ["2 personas"] }], "notes": "Hablar despacio" },
                { "title": "Comparación", "left": { "title": "Antes", "bullets": ["lento"] }, "right": { "bullets": ["rápido"] } }
            ] }),
            &tmp.0,
            &access,
        )
        .unwrap();
        let text = read_document("p.pptx", &access).unwrap();
        for needle in ["## Diapositiva 1\nPlan 2027\nEquipo", "## Diapositiva 2\nMetas\nCrecer\nContratar\n2 personas", "Notas: Hablar despacio", "Antes\nlento", "rápido"] {
            assert!(text.contains(needle), "{needle}\n{text}");
        }
        assert!(!text.contains("‹#›") && !text.lines().any(|l| l == "2"), "slide numbers are left out:\n{text}");
    }

    #[test]
    fn text_files_caps_and_refusals() {
        let tmp = TempDir::new("read-text");
        let access = Access::new(&tmp.0, []);
        std::fs::write(tmp.0.join("a.csv"), "\u{FEFF}a,b\n1,2\n").unwrap();
        std::fs::write(tmp.0.join("latin.txt"), [b'a', 0xF1, b'o', 0x80]).unwrap();
        std::fs::write(tmp.0.join("big.md"), "x".repeat(MAX_CHARS + 50)).unwrap();
        std::fs::write(tmp.0.join("viejo.doc"), "x").unwrap();
        std::fs::write(tmp.0.join("raro.bin"), "x").unwrap();
        std::fs::write(tmp.0.join("falso.docx"), "no soy zip").unwrap();
        assert!(read_document("a.csv", &access).unwrap().ends_with("a,b\n1,2"));
        assert!(read_document("latin.txt", &access).unwrap().ends_with("año€"));
        let big = read_document("big.md", &access).unwrap();
        assert!(big.contains("[Recortado: se muestran los primeros 200000 caracteres de 200050."));
        assert!(read_document("viejo.doc", &access).unwrap_err().0.contains("97-2003"));
        assert!(read_document("raro.bin", &access).is_err());
        assert!(read_document("falso.docx", &access).unwrap_err().0.contains("ZIP"));
        assert!(read_document("/etc/hosts", &access).unwrap_err().0.contains("permiso"));
    }
}
