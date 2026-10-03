//! The text layer of a PDF, without any PDF library: the objects are found by scanning the file (so a broken cross
//! reference table does not matter), object streams are unpacked, the page tree is walked in order, and each
//! page's content stream is interpreted just enough to place its text: fonts with their ToUnicode maps, widths and
//! simple encodings, the text and graphics matrices, form XObjects. Filters: FlateDecode (miniz_oxide),
//! ASCIIHexDecode and ASCII85Decode; streams with any other filter are skipped. Encrypted PDFs are refused. A scanned
//! PDF has no text layer, and says so.
//!
//! Every size is capped (streams, objects, nesting, pages, output), so a hostile file costs bounded time and memory.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;

use super::OfficeError;
use super::read::cp1252;

/// Largest single stream once decoded, and all of them together.
const MAX_STREAM: usize = 32 * 1024 * 1024;
const MAX_INFLATED: usize = 256 * 1024 * 1024;
const MAX_DEPTH: usize = 48;
const MAX_OBJECTS: usize = 500_000;
const MAX_PAGES: usize = 5000;
const MAX_FORM_DEPTH: usize = 6;

type Dict = HashMap<String, Obj>;

#[derive(Clone, Debug, PartialEq)]
enum Obj {
    Null,
    Bool(bool),
    Num(f64),
    Str(Vec<u8>),
    Name(String),
    Array(Vec<Obj>),
    Dict(Dict),
    Ref(u32),
    /// A stream: its dictionary and where its raw bytes sit in the file.
    Stream(Dict, Range<usize>),
    /// A bare keyword: an operator in a content stream.
    Op(Vec<u8>),
}

impl Obj {
    fn num(&self) -> Option<f64> {
        if let Obj::Num(n) = self { Some(*n) } else { None }
    }
    fn name(&self) -> Option<&str> {
        if let Obj::Name(n) = self { Some(n) } else { None }
    }
    fn dict(&self) -> Option<&Dict> {
        match self {
            Obj::Dict(d) | Obj::Stream(d, _) => Some(d),
            _ => None,
        }
    }
}

// MARK: lexer

fn is_ws(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}

fn is_delim(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

struct Lexer<'a> {
    d: &'a [u8],
    i: usize,
}

impl<'a> Lexer<'a> {
    fn new(d: &'a [u8], i: usize) -> Self {
        Lexer { d, i }
    }

    fn skip_ws(&mut self) {
        while self.i < self.d.len() {
            if is_ws(self.d[self.i]) {
                self.i += 1;
            } else if self.d[self.i] == b'%' {
                while self.i < self.d.len() && self.d[self.i] != b'\n' && self.d[self.i] != b'\r' {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
    }

    fn at_end(&mut self) -> bool {
        self.skip_ws();
        self.i >= self.d.len()
    }

    fn regular(&mut self) -> &'a [u8] {
        let start = self.i;
        while self.i < self.d.len() && !is_ws(self.d[self.i]) && !is_delim(self.d[self.i]) {
            self.i += 1;
        }
        &self.d[start..self.i]
    }

    fn literal(&mut self) -> Vec<u8> {
        // After the opening parenthesis.
        let mut out = Vec::new();
        let mut depth = 1;
        while self.i < self.d.len() {
            let b = self.d[self.i];
            self.i += 1;
            match b {
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(b);
                }
                b'\\' if self.i < self.d.len() => {
                    let e = self.d[self.i];
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'0'..=b'7' => {
                            let mut v = (e - b'0') as u32;
                            for _ in 0..2 {
                                match self.d.get(self.i) {
                                    Some(&c @ b'0'..=b'7') => {
                                        v = v * 8 + (c - b'0') as u32;
                                        self.i += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(v as u8);
                        }
                        b'\r' => {
                            if self.d.get(self.i) == Some(&b'\n') {
                                self.i += 1;
                            }
                        }
                        b'\n' => {}
                        other => out.push(other),
                    }
                }
                other => out.push(other),
            }
        }
        out
    }

    fn hex(&mut self) -> Vec<u8> {
        // After the opening angle bracket.
        let mut digits = Vec::new();
        while self.i < self.d.len() && self.d[self.i] != b'>' {
            let c = self.d[self.i];
            if let Some(v) = (c as char).to_digit(16) {
                digits.push(v as u8);
            }
            self.i += 1;
        }
        self.i = (self.i + 1).min(self.d.len());
        if digits.len() % 2 == 1 {
            digits.push(0);
        }
        digits.chunks(2).map(|p| p[0] * 16 + p[1]).collect()
    }

    fn name(&mut self) -> String {
        // After the slash.
        let raw = self.regular();
        let mut out = Vec::with_capacity(raw.len());
        let mut k = 0;
        while k < raw.len() {
            if raw[k] == b'#'
                && k + 2 < raw.len()
                && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&raw[k + 1..k + 3]).unwrap_or("zz"), 16)
            {
                out.push(v);
                k += 3;
                continue;
            }
            out.push(raw[k]);
            k += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn integer_ahead(&mut self) -> Option<u32> {
        self.skip_ws();
        let start = self.i;
        let word = self.regular();
        if !word.is_empty() && word.iter().all(u8::is_ascii_digit) {
            std::str::from_utf8(word).ok()?.parse().ok()
        } else {
            self.i = start;
            None
        }
    }

    /// The next object (or operator), `None` at the end or on a stray closing bracket.
    fn object(&mut self, depth: usize) -> Option<Obj> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.skip_ws();
        let &b = self.d.get(self.i)?;
        match b {
            b'(' => {
                self.i += 1;
                Some(Obj::Str(self.literal()))
            }
            b'<' if self.d.get(self.i + 1) == Some(&b'<') => {
                self.i += 2;
                let mut dict = Dict::new();
                loop {
                    self.skip_ws();
                    if self.i >= self.d.len() {
                        break;
                    }
                    if self.d[self.i..].starts_with(b">>") {
                        self.i += 2;
                        break;
                    }
                    if self.d[self.i] == b'/' {
                        self.i += 1;
                        let key = self.name();
                        let value = self.object(depth + 1).unwrap_or(Obj::Null);
                        dict.insert(key, value);
                    } else if self.object(depth + 1).is_none() {
                        // Garbage that is not even an object: skip one byte.
                        self.i += 1;
                    }
                }
                Some(Obj::Dict(dict))
            }
            b'<' => {
                self.i += 1;
                Some(Obj::Str(self.hex()))
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws();
                    match self.d.get(self.i) {
                        None => break,
                        Some(b']') => {
                            self.i += 1;
                            break;
                        }
                        Some(_) => {
                            if depth >= MAX_DEPTH {
                                return None;
                            }
                            match self.object(depth + 1) {
                                Some(o) => items.push(o),
                                None => self.i += 1,
                            }
                        }
                    }
                }
                Some(Obj::Array(items))
            }
            b'/' => {
                self.i += 1;
                Some(Obj::Name(self.name()))
            }
            b']' | b'>' | b')' => None,
            b'{' | b'}' => {
                self.i += 1;
                Some(Obj::Op(vec![b]))
            }
            _ => {
                let word = self.regular();
                if word.is_empty() {
                    self.i += 1;
                    return Some(Obj::Op(vec![b]));
                }
                let text = std::str::from_utf8(word).unwrap_or("");
                if let Ok(n) = text.parse::<f64>() {
                    // `12 0 R` is a reference.
                    if word.iter().all(u8::is_ascii_digit) {
                        let save = self.i;
                        if self.integer_ahead().is_some() {
                            self.skip_ws();
                            if self.d[self.i..].starts_with(b"R") && self.d.get(self.i + 1).is_none_or(|&c| is_ws(c) || is_delim(c)) {
                                self.i += 1;
                                return Some(Obj::Ref(n as u32));
                            }
                        }
                        self.i = save;
                    }
                    return Some(Obj::Num(n));
                }
                Some(match word {
                    b"true" => Obj::Bool(true),
                    b"false" => Obj::Bool(false),
                    b"null" => Obj::Null,
                    _ => Obj::Op(word.to_vec()),
                })
            }
        }
    }
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= hay.len() {
        return None;
    }
    hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

// MARK: document

struct Document<'a> {
    data: &'a [u8],
    objects: HashMap<u32, Obj>,
    inflated: std::cell::Cell<usize>,
}

/// Where the `num gen obj` before `at` (the position of `obj`) starts, and its number.
fn object_header(d: &[u8], at: usize) -> Option<u32> {
    let mut k = at;
    let back_ws = |k: &mut usize| {
        let start = *k;
        while *k > 0 && is_ws(d[*k - 1]) {
            *k -= 1;
        }
        *k < start
    };
    let back_digits = |k: &mut usize| {
        let end = *k;
        while *k > 0 && d[*k - 1].is_ascii_digit() {
            *k -= 1;
        }
        (*k < end).then_some((*k, end))
    };
    if !back_ws(&mut k) {
        return None;
    }
    back_digits(&mut k)?;
    if !back_ws(&mut k) {
        return None;
    }
    let (start, end) = back_digits(&mut k)?;
    if start > 0 && !is_ws(d[start - 1]) && !is_delim(d[start - 1]) {
        return None;
    }
    std::str::from_utf8(&d[start..end]).ok()?.parse().ok()
}

impl<'a> Document<'a> {
    fn parse(data: &'a [u8]) -> Result<Self, OfficeError> {
        if find(&data[..data.len().min(1024)], b"%PDF", 0).is_none() {
            return Err(OfficeError::new("El archivo no es un PDF válido."));
        }
        let mut doc = Document { data, objects: HashMap::new(), inflated: std::cell::Cell::new(0) };
        let mut i = 0;
        while let Some(at) = find(data, b"obj", i) {
            i = at + 3;
            if data.get(at + 3).is_some_and(|&c| !is_ws(c) && !is_delim(c)) {
                continue;
            }
            let Some(num) = object_header(data, at) else { continue };
            let mut lx = Lexer::new(data, at + 3);
            let Some(obj) = lx.object(0) else { continue };
            lx.skip_ws();
            let obj = match obj {
                Obj::Dict(dict) if data[lx.i..].starts_with(b"stream") => {
                    let mut start = lx.i + 6;
                    if data.get(start) == Some(&b'\r') {
                        start += 1;
                    }
                    if data.get(start) == Some(&b'\n') {
                        start += 1;
                    }
                    let declared = dict.get("Length").and_then(Obj::num).map(|n| n as usize);
                    let end = declared
                        .filter(|&len| {
                            let mut e = start + len;
                            while e < data.len() && is_ws(data[e]) {
                                e += 1;
                            }
                            data[e.min(data.len())..].starts_with(b"endstream")
                        })
                        .map(|len| start + len)
                        .or_else(|| {
                            find(data, b"endstream", start).map(|mut e| {
                                while e > start && (data[e - 1] == b'\n' || data[e - 1] == b'\r') {
                                    e -= 1;
                                }
                                e
                            })
                        })
                        .unwrap_or(data.len());
                    i = end;
                    Obj::Stream(dict, start..end.min(data.len()))
                }
                other => {
                    i = lx.i.max(i);
                    other
                }
            };
            doc.objects.insert(num, obj);
            if doc.objects.len() > MAX_OBJECTS {
                break;
            }
        }
        if doc.encrypted() {
            return Err(OfficeError::new(
                "El PDF está cifrado (protegido) y no puedo leer su texto. Si el modelo puede abrir PDFs directamente, que lo lea así.",
            ));
        }
        doc.unpack_object_streams();
        Ok(doc)
    }

    fn encrypted(&self) -> bool {
        let mut trailers: Vec<Dict> = Vec::new();
        let mut i = 0;
        while let Some(at) = find(self.data, b"trailer", i) {
            i = at + 7;
            if let Some(Obj::Dict(d)) = Lexer::new(self.data, at + 7).object(0) {
                trailers.push(d);
            }
        }
        trailers.iter().chain(self.objects.values().filter_map(|o| match o {
            Obj::Stream(d, _) if d.get("Type").and_then(Obj::name) == Some("XRef") => Some(d),
            _ => None,
        }))
        .any(|d| d.contains_key("Encrypt"))
    }

    fn unpack_object_streams(&mut self) {
        let streams: Vec<(Dict, Range<usize>)> = self
            .objects
            .values()
            .filter_map(|o| match o {
                Obj::Stream(d, r) if d.get("Type").and_then(Obj::name) == Some("ObjStm") => Some((d.clone(), r.clone())),
                _ => None,
            })
            .collect();
        for (dict, range) in streams {
            let Some(data) = self.decode(&dict, range) else { continue };
            let n = dict.get("N").and_then(Obj::num).unwrap_or(0.0) as usize;
            let first = dict.get("First").and_then(Obj::num).unwrap_or(0.0) as usize;
            if first > data.len() {
                continue;
            }
            let mut header = Lexer::new(&data[..first], 0);
            let mut entries = Vec::new();
            for _ in 0..n.min(100_000) {
                match (header.integer_ahead(), header.integer_ahead()) {
                    (Some(num), Some(offset)) => entries.push((num, offset as usize)),
                    _ => break,
                }
            }
            for (num, offset) in entries {
                if self.objects.contains_key(&num) || self.objects.len() > MAX_OBJECTS {
                    continue;
                }
                if let Some(obj) = Lexer::new(&data, first + offset).object(0) {
                    self.objects.insert(num, obj);
                }
            }
        }
    }

    fn resolve<'b>(&'b self, obj: &'b Obj) -> &'b Obj {
        let mut current = obj;
        for _ in 0..16 {
            match current {
                Obj::Ref(n) => current = self.objects.get(n).unwrap_or(&Obj::Null),
                other => return other,
            }
        }
        &Obj::Null
    }

    fn get<'b>(&'b self, dict: &'b Dict, key: &str) -> &'b Obj {
        dict.get(key).map(|o| self.resolve(o)).unwrap_or(&Obj::Null)
    }

    /// A stream's bytes with its filters undone, or `None` for a filter we do not know.
    fn decode(&self, dict: &Dict, range: Range<usize>) -> Option<Vec<u8>> {
        let mut data = self.data.get(range)?.to_vec();
        let filters: Vec<String> = match self.get(dict, "Filter") {
            Obj::Name(n) => vec![n.clone()],
            Obj::Array(items) => items.iter().filter_map(|f| self.resolve(f).name().map(str::to_string)).collect(),
            _ => vec![],
        };
        for filter in filters {
            data = match filter.as_str() {
                "FlateDecode" | "Fl" => {
                    let budget = MAX_STREAM.min(MAX_INFLATED.saturating_sub(self.inflated.get()));
                    if budget == 0 {
                        return None;
                    }
                    let out = match miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&data, budget) {
                        Ok(out) => out,
                        Err(err) if !err.output.is_empty() => err.output,
                        Err(_) => miniz_oxide::inflate::decompress_to_vec_with_limit(&data, budget).ok()?,
                    };
                    self.inflated.set(self.inflated.get() + out.len());
                    out
                }
                "ASCIIHexDecode" | "AHx" => Lexer::new(&data, 0).hex(),
                "ASCII85Decode" | "A85" => ascii85(&data)?,
                _ => return None,
            };
        }
        Some(data)
    }

    fn stream_bytes(&self, obj: &Obj) -> Option<Vec<u8>> {
        match self.resolve(obj) {
            Obj::Stream(d, r) => self.decode(d, r.clone()),
            _ => None,
        }
    }

    /// The pages in reading order, each with the resources it inherits.
    fn pages(&self) -> Vec<(Dict, Option<Dict>)> {
        let mut root = None;
        let mut i = 0;
        while let Some(at) = find(self.data, b"trailer", i) {
            i = at + 7;
            if let Some(Obj::Dict(d)) = Lexer::new(self.data, at + 7).object(0)
                && let Some(r) = d.get("Root")
            {
                root = Some(r.clone());
            }
        }
        if root.is_none() {
            root = self.objects.values().find_map(|o| match o {
                Obj::Stream(d, _) if d.get("Type").and_then(Obj::name) == Some("XRef") => d.get("Root").cloned(),
                _ => None,
            });
        }
        let catalog = root.as_ref().map(|r| self.resolve(r)).and_then(Obj::dict).cloned().or_else(|| {
            let mut nums: Vec<&u32> = self.objects.keys().collect();
            nums.sort();
            nums.into_iter().rev().find_map(|n| {
                let d = self.objects[n].dict()?;
                (d.get("Type").and_then(Obj::name) == Some("Catalog")).then(|| d.clone())
            })
        });
        let mut out = Vec::new();
        if let Some(catalog) = catalog {
            let mut seen = HashSet::new();
            self.walk(catalog.get("Pages").unwrap_or(&Obj::Null), None, 0, &mut seen, &mut out);
        }
        if out.is_empty() {
            let mut nums: Vec<&u32> = self.objects.keys().collect();
            nums.sort();
            for n in nums {
                if let Some(d) = self.objects[n].dict()
                    && d.get("Type").and_then(Obj::name) == Some("Page")
                {
                    let resources = self.get(d, "Resources").dict().cloned();
                    out.push((d.clone(), resources));
                }
            }
        }
        out.truncate(MAX_PAGES);
        out
    }

    fn walk(&self, node: &Obj, inherited: Option<Dict>, depth: usize, seen: &mut HashSet<u32>, out: &mut Vec<(Dict, Option<Dict>)>) {
        if depth > 64 || out.len() >= MAX_PAGES {
            return;
        }
        if let Obj::Ref(n) = node
            && !seen.insert(*n)
        {
            return;
        }
        let Some(dict) = self.resolve(node).dict() else { return };
        let resources = self.get(dict, "Resources").dict().cloned().or(inherited);
        match self.get(dict, "Kids") {
            Obj::Array(kids) if dict.get("Type").and_then(Obj::name) != Some("Page") => {
                for kid in kids {
                    self.walk(kid, resources.clone(), depth + 1, seen, out);
                }
            }
            _ => out.push((dict.clone(), resources)),
        }
    }
}

fn ascii85(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut group = [0u8; 5];
    let mut n = 0;
    let body = data.strip_prefix(b"<~").unwrap_or(data);
    for &c in body {
        match c {
            b'~' => break,
            b'z' if n == 0 => out.extend_from_slice(&[0, 0, 0, 0]),
            b'!'..=b'u' => {
                group[n] = c - b'!';
                n += 1;
                if n == 5 {
                    let v = group.iter().fold(0u64, |acc, &d| acc * 85 + d as u64);
                    out.extend_from_slice(&(v as u32).to_be_bytes());
                    n = 0;
                }
            }
            c if is_ws(c) => {}
            _ => return None,
        }
    }
    if n > 1 {
        for slot in group.iter_mut().skip(n) {
            *slot = 84;
        }
        let v = group.iter().fold(0u64, |acc, &d| acc * 85 + d as u64);
        out.extend_from_slice(&(v as u32).to_be_bytes()[..n - 1]);
    }
    Some(out)
}

// MARK: fonts

/// Mac OS Roman, 0x80 to 0xFF.
const MAC_ROMAN: [&str; 8] = [
    "ÄÅÇÉÑÖÜáàâäãåçéè",
    "êëíìîïñóòôöõúùûü",
    "†°¢£§•¶ß®©™´¨≠ÆØ",
    "∞±≤≥¥µ∂∑∏π∫ªºΩæø",
    "¿¡¬√ƒ≈∆«»…\u{A0}ÀÃÕŒœ",
    "–—“”‘’÷◊ÿŸ⁄€‹›ﬁﬂ",
    "‡·‚„‰ÂÊÁËÈÍÎÏÌÓÔ",
    "\u{F8FF}ÒÚÛÙıˆ˜¯˘˙˚¸˝˛ˇ",
];

fn mac_roman(b: u8) -> char {
    if b < 0x80 {
        return b as char;
    }
    let row = MAC_ROMAN[((b - 0x80) / 16) as usize];
    row.chars().nth(((b - 0x80) % 16) as usize).unwrap_or('?')
}

/// A glyph name from an encoding's /Differences as a character, for the names text fonts actually use.
fn glyph_char(name: &str) -> Option<char> {
    if let Some(hex) = name.strip_prefix("uni").filter(|h| h.len() == 4) {
        return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
    }
    if let Some(hex) = name.strip_prefix('u').filter(|h| (4..=6).contains(&h.len())) {
        return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
    }
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next())
        && c.is_ascii_alphabetic()
    {
        return Some(c);
    }
    const NAMES: &[(&str, char)] = &[
        ("space", ' '), ("exclam", '!'), ("quotedbl", '"'), ("numbersign", '#'), ("dollar", '$'), ("percent", '%'),
        ("ampersand", '&'), ("quotesingle", '\''), ("quoteright", '’'), ("quoteleft", '‘'), ("parenleft", '('),
        ("parenright", ')'), ("asterisk", '*'), ("plus", '+'), ("comma", ','), ("hyphen", '-'), ("period", '.'),
        ("slash", '/'), ("zero", '0'), ("one", '1'), ("two", '2'), ("three", '3'), ("four", '4'), ("five", '5'),
        ("six", '6'), ("seven", '7'), ("eight", '8'), ("nine", '9'), ("colon", ':'), ("semicolon", ';'), ("less", '<'),
        ("equal", '='), ("greater", '>'), ("question", '?'), ("at", '@'), ("bracketleft", '['), ("backslash", '\\'),
        ("bracketright", ']'), ("underscore", '_'), ("braceleft", '{'), ("bar", '|'), ("braceright", '}'),
        ("aacute", 'á'), ("eacute", 'é'), ("iacute", 'í'), ("oacute", 'ó'), ("uacute", 'ú'), ("Aacute", 'Á'),
        ("Eacute", 'É'), ("Iacute", 'Í'), ("Oacute", 'Ó'), ("Uacute", 'Ú'), ("ntilde", 'ñ'), ("Ntilde", 'Ñ'),
        ("udieresis", 'ü'), ("Udieresis", 'Ü'), ("ccedilla", 'ç'), ("agrave", 'à'), ("egrave", 'è'), ("questiondown", '¿'),
        ("exclamdown", '¡'), ("endash", '–'), ("emdash", '—'), ("bullet", '•'), ("quotedblleft", '“'),
        ("quotedblright", '”'), ("ellipsis", '…'), ("degree", '°'), ("ordfeminine", 'ª'), ("ordmasculine", 'º'),
        ("Euro", '€'), ("copyright", '©'), ("registered", '®'), ("trademark", '™'), ("fi", 'ﬁ'), ("fl", 'ﬂ'),
        ("minus", '−'), ("multiply", '×'), ("divide", '÷'), ("section", '§'), ("paragraph", '¶'), ("nbspace", '\u{A0}'),
    ];
    NAMES.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

#[derive(Default)]
struct Font {
    /// ToUnicode: code → text, and how many bytes a code takes.
    unicode: HashMap<u32, String>,
    code_bytes: usize,
    /// A composite (Type0) font: two-byte codes unless its ToUnicode map says otherwise.
    composite: bool,
    mac: bool,
    differences: HashMap<u8, char>,
    /// Widths in thousandths of the font size.
    widths: HashMap<u32, f64>,
    default_width: f64,
}

impl Font {
    fn bytes_per_code(&self) -> usize {
        if self.code_bytes > 0 {
            self.code_bytes
        } else if self.composite {
            2
        } else {
            1
        }
    }

    /// Each code of `bytes`: its text and its width.
    fn codes(&self, bytes: &[u8]) -> Vec<(u32, String, f64)> {
        let step = self.bytes_per_code();
        bytes
            .chunks(step)
            .map(|chunk| {
                let code = chunk.iter().fold(0u32, |acc, &b| (acc << 8) | b as u32);
                let text = match self.unicode.get(&code) {
                    Some(t) => t.clone(),
                    None if step == 1 => {
                        let b = code as u8;
                        let c = self.differences.get(&b).copied().unwrap_or_else(|| if self.mac { mac_roman(b) } else { cp1252(b) });
                        c.to_string()
                    }
                    None => String::new(),
                };
                let width = self.widths.get(&code).copied().unwrap_or(self.default_width);
                (code, text, width)
            })
            .collect()
    }
}

impl Document<'_> {
    fn font(&self, dict: &Dict) -> Font {
        let mut font = Font { default_width: 500.0, ..Font::default() };
        font.composite = self.get(dict, "Subtype").name() == Some("Type0");
        if let Some(cmap) = self.stream_bytes(dict.get("ToUnicode").unwrap_or(&Obj::Null)) {
            parse_cmap(&cmap, &mut font);
        }
        match self.get(dict, "Encoding") {
            Obj::Name(n) => font.mac = n == "MacRomanEncoding",
            Obj::Dict(enc) => {
                font.mac = self.get(enc, "BaseEncoding").name() == Some("MacRomanEncoding");
                if let Obj::Array(items) = self.get(enc, "Differences") {
                    let mut code = 0u32;
                    for item in items {
                        match self.resolve(item) {
                            Obj::Num(n) => code = *n as u32,
                            Obj::Name(name) => {
                                if let (Ok(b), Some(c)) = (u8::try_from(code), glyph_char(name)) {
                                    font.differences.insert(b, c);
                                }
                                code += 1;
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
        if font.composite {
            font.default_width = 1000.0;
            if let Obj::Array(descendants) = self.get(dict, "DescendantFonts")
                && let Some(cid) = descendants.first().map(|d| self.resolve(d)).and_then(Obj::dict)
            {
                if let Some(dw) = self.get(cid, "DW").num() {
                    font.default_width = dw;
                }
                if let Obj::Array(w) = self.get(cid, "W") {
                    let mut k = 0;
                    while k < w.len() && font.widths.len() < 65_536 {
                        let first = self.resolve(&w[k]).num().unwrap_or(0.0) as u32;
                        match w.get(k + 1).map(|o| self.resolve(o)) {
                            Some(Obj::Array(list)) => {
                                for (j, v) in list.iter().enumerate().take(65_536) {
                                    font.widths.insert(first + j as u32, self.resolve(v).num().unwrap_or(font.default_width));
                                }
                                k += 2;
                            }
                            Some(Obj::Num(last)) => {
                                let width = w.get(k + 2).and_then(|o| self.resolve(o).num()).unwrap_or(font.default_width);
                                for code in first..=(*last as u32).min(first + 65_535) {
                                    font.widths.insert(code, width);
                                }
                                k += 3;
                            }
                            _ => break,
                        }
                    }
                }
            }
        } else {
            let first = self.get(dict, "FirstChar").num().unwrap_or(0.0) as u32;
            if let Obj::Array(widths) = self.get(dict, "Widths") {
                for (j, v) in widths.iter().enumerate().take(256) {
                    font.widths.insert(first + j as u32, self.resolve(v).num().unwrap_or(500.0));
                }
            }
            if let Some(desc) = self.get(dict, "FontDescriptor").dict()
                && let Some(missing) = self.get(desc, "MissingWidth").num().filter(|m| *m > 0.0)
            {
                font.default_width = missing;
            }
        }
        font
    }
}

fn utf16be(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.chunks(2).map(|p| u16::from_be_bytes([p[0], *p.get(1).unwrap_or(&0)])).collect();
    String::from_utf16_lossy(&units)
}

fn code_of(bytes: &[u8]) -> u32 {
    bytes.iter().take(4).fold(0u32, |acc, &b| (acc << 8) | b as u32)
}

/// A ToUnicode CMap: `bfchar` and `bfrange` entries, and the code width from `codespacerange`.
fn parse_cmap(data: &[u8], font: &mut Font) {
    let mut lx = Lexer::new(data, 0);
    let mut section = "";
    let mut pending: Vec<Obj> = Vec::new();
    while !lx.at_end() && font.unicode.len() < 200_000 {
        let Some(obj) = lx.object(0) else {
            lx.i += 1;
            continue;
        };
        if let Obj::Op(op) = &obj {
            section = match op.as_slice() {
                b"begincodespacerange" => "space",
                b"beginbfchar" => "char",
                b"beginbfrange" => "range",
                _ => "",
            };
            pending.clear();
            continue;
        }
        pending.push(obj);
        match section {
            "space" if pending.len() == 2 => {
                if let Obj::Str(lo) = &pending[0]
                    && font.code_bytes == 0
                {
                    font.code_bytes = lo.len().clamp(1, 4);
                }
                pending.clear();
            }
            "char" if pending.len() == 2 => {
                if let Obj::Str(src) = &pending[0] {
                    if font.code_bytes == 0 {
                        font.code_bytes = src.len().clamp(1, 4);
                    }
                    let text = match &pending[1] {
                        Obj::Str(dst) => utf16be(dst),
                        Obj::Name(n) => glyph_char(n).map(String::from).unwrap_or_default(),
                        _ => String::new(),
                    };
                    font.unicode.insert(code_of(src), text);
                }
                pending.clear();
            }
            "range" if pending.len() == 3 => {
                if let (Obj::Str(lo), Obj::Str(hi)) = (&pending[0], &pending[1]) {
                    if font.code_bytes == 0 {
                        font.code_bytes = lo.len().clamp(1, 4);
                    }
                    let (lo, hi) = (code_of(lo), code_of(hi));
                    if hi >= lo && hi - lo <= 65_535 {
                        match &pending[2] {
                            Obj::Str(dst) => {
                                let mut units: Vec<u16> = dst.chunks(2).map(|p| u16::from_be_bytes([p[0], *p.get(1).unwrap_or(&0)])).collect();
                                for code in lo..=hi {
                                    font.unicode.insert(code, String::from_utf16_lossy(&units));
                                    if let Some(last) = units.last_mut() {
                                        *last = last.wrapping_add(1);
                                    }
                                }
                            }
                            Obj::Array(items) => {
                                for (k, item) in items.iter().enumerate().take((hi - lo + 1) as usize) {
                                    if let Obj::Str(dst) = item {
                                        font.unicode.insert(lo + k as u32, utf16be(dst));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                pending.clear();
            }
            "" => pending.clear(),
            _ => {}
        }
    }
}

// MARK: content

#[derive(Clone, Copy, Debug, PartialEq)]
struct Matrix([f64; 6]);

impl Matrix {
    const IDENTITY: Matrix = Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    /// `self × other`.
    fn times(self, other: Matrix) -> Matrix {
        let [a1, b1, c1, d1, e1, f1] = self.0;
        let [a2, b2, c2, d2, e2, f2] = other.0;
        Matrix([
            a1 * a2 + b1 * c2,
            a1 * b2 + b1 * d2,
            c1 * a2 + d1 * c2,
            c1 * b2 + d1 * d2,
            e1 * a2 + f1 * c2 + e2,
            e1 * b2 + f1 * d2 + f2,
        ])
    }

    fn translate(tx: f64, ty: f64) -> Matrix {
        Matrix([1.0, 0.0, 0.0, 1.0, tx, ty])
    }

    fn from(stack: &[Obj]) -> Option<Matrix> {
        let n: Vec<f64> = stack.iter().rev().take(6).filter_map(Obj::num).collect();
        (n.len() == 6).then(|| Matrix([n[5], n[4], n[3], n[2], n[1], n[0]]))
    }
}

/// Text state while a content stream runs, and where the last text ended.
struct Painter<'d, 'a> {
    doc: &'d Document<'a>,
    fonts: HashMap<u32, Rc<Font>>,
    out: String,
    max: usize,
    /// Where the last shown text ended (device space) and how big it was.
    last: Option<(f64, f64, f64)>,
    unmapped: bool,
}

struct State {
    ctm: Matrix,
    tm: Matrix,
    tlm: Matrix,
    font: Option<Rc<Font>>,
    size: f64,
    char_spacing: f64,
    word_spacing: f64,
    scale: f64,
    leading: f64,
}

impl State {
    fn new(ctm: Matrix) -> Self {
        State {
            ctm,
            tm: Matrix::IDENTITY,
            tlm: Matrix::IDENTITY,
            font: None,
            size: 12.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            scale: 1.0,
            leading: 0.0,
        }
    }
}

impl<'d, 'a> Painter<'d, 'a> {
    fn font(&mut self, resources: Option<&Dict>, name: &str) -> Option<Rc<Font>> {
        let fonts = self.doc.get(resources?, "Font").dict()?;
        let entry = fonts.get(name)?;
        if let Obj::Ref(n) = entry {
            if let Some(f) = self.fonts.get(n) {
                return Some(f.clone());
            }
            let font = Rc::new(self.doc.font(self.doc.resolve(entry).dict()?));
            self.fonts.insert(*n, font.clone());
            return Some(font);
        }
        Some(Rc::new(self.doc.font(self.doc.resolve(entry).dict()?)))
    }

    fn show(&mut self, state: &mut State, bytes: &[u8]) {
        let Some(font) = state.font.clone() else { return };
        let trm = state.tm.times(state.ctm);
        let size = (state.size * (trm.0[2].hypot(trm.0[3]))).abs().max(0.1);
        let (x, y) = (trm.0[4], trm.0[5]);
        let single = font.bytes_per_code() == 1;
        let mut text = String::new();
        for (code, piece, width) in font.codes(bytes) {
            if piece.is_empty() && !single {
                self.unmapped = true;
            }
            text.push_str(&piece);
            let spacing = if single && code == 32 { state.word_spacing } else { 0.0 };
            let advance = (width / 1000.0 * state.size + state.char_spacing + spacing) * state.scale;
            state.tm = Matrix::translate(advance, 0.0).times(state.tm);
        }
        let text: String = text.chars().filter(|c| !c.is_control() || *c == '\t').collect();
        let end = state.tm.times(state.ctm);
        if text.is_empty() {
            return;
        }
        if let Some((last_x, last_y, last_size)) = self.last {
            let line = size.max(last_size);
            if (y - last_y).abs() > line * 0.6 {
                if !self.out.ends_with('\n') {
                    self.out.push('\n');
                }
                if (y - last_y).abs() > line * 2.2 {
                    self.out.push('\n');
                }
            } else if (x - last_x > line * 0.18 || last_x - x > line * 2.0) && !self.out.ends_with([' ', '\n']) && !text.starts_with(' ') {
                self.out.push(' ');
            }
        }
        self.out.push_str(&text);
        self.last = Some((end.0[4], y, size));
    }

    fn run(&mut self, content: &[u8], resources: Option<&Dict>, ctm: Matrix, depth: usize) {
        let mut lx = Lexer::new(content, 0);
        let mut stack: Vec<Obj> = Vec::new();
        let mut state = State::new(ctm);
        let mut saved: Vec<Matrix> = Vec::new();
        while !lx.at_end() && self.out.len() <= self.max * 4 {
            let Some(obj) = lx.object(0) else {
                lx.i += 1;
                continue;
            };
            let Obj::Op(op) = obj else {
                if stack.len() < 64 {
                    stack.push(obj);
                }
                continue;
            };
            let num = |k: usize| stack.iter().rev().nth(k).and_then(Obj::num).unwrap_or(0.0);
            match op.as_slice() {
                b"q" => {
                    if saved.len() < 64 {
                        saved.push(state.ctm);
                    }
                }
                b"Q" => state.ctm = saved.pop().unwrap_or(state.ctm),
                b"cm" => {
                    if let Some(m) = Matrix::from(&stack) {
                        state.ctm = m.times(state.ctm);
                    }
                }
                b"BT" => {
                    state.tm = Matrix::IDENTITY;
                    state.tlm = Matrix::IDENTITY;
                }
                b"Tf" => {
                    state.size = num(0);
                    if let Some(Obj::Name(name)) = stack.iter().rev().nth(1) {
                        let name = name.clone();
                        state.font = self.font(resources, &name);
                    }
                }
                b"Tc" => state.char_spacing = num(0),
                b"Tw" => state.word_spacing = num(0),
                b"Tz" => state.scale = num(0) / 100.0,
                b"TL" => state.leading = num(0),
                b"Td" | b"TD" => {
                    let (tx, ty) = (num(1), num(0));
                    if op == b"TD" {
                        state.leading = -ty;
                    }
                    state.tlm = Matrix::translate(tx, ty).times(state.tlm);
                    state.tm = state.tlm;
                }
                b"Tm" => {
                    if let Some(m) = Matrix::from(&stack) {
                        state.tlm = m;
                        state.tm = m;
                    }
                }
                b"T*" => {
                    state.tlm = Matrix::translate(0.0, -state.leading).times(state.tlm);
                    state.tm = state.tlm;
                }
                b"Tj" | b"'" | b"\"" => {
                    if op != b"Tj" {
                        if op == b"\"" {
                            state.word_spacing = num(2);
                            state.char_spacing = num(1);
                        }
                        state.tlm = Matrix::translate(0.0, -state.leading).times(state.tlm);
                        state.tm = state.tlm;
                    }
                    if let Some(Obj::Str(s)) = stack.last() {
                        let s = s.clone();
                        self.show(&mut state, &s);
                    }
                }
                b"TJ" => {
                    if let Some(Obj::Array(items)) = stack.last() {
                        let items = items.clone();
                        for item in items {
                            match item {
                                Obj::Str(s) => self.show(&mut state, &s),
                                Obj::Num(n) => {
                                    let tx = -n / 1000.0 * state.size * state.scale;
                                    state.tm = Matrix::translate(tx, 0.0).times(state.tm);
                                }
                                _ => {}
                            }
                        }
                    }
                }
                b"ID" => {
                    // Inline image data: skip to the EI that ends it.
                    let mut k = lx.i + 1;
                    while let Some(at) = find(content, b"EI", k) {
                        let before = at == 0 || is_ws(content[at - 1]);
                        let after = content.get(at + 2).is_none_or(|&c| is_ws(c));
                        if before && after {
                            break;
                        }
                        k = at + 2;
                    }
                    lx.i = find(content, b"EI", k).map(|p| p + 2).unwrap_or(content.len());
                }
                b"Do" if depth < MAX_FORM_DEPTH => {
                    if let (Some(Obj::Name(name)), Some(res)) = (stack.last(), resources)
                        && let Some(xobjects) = self.doc.get(res, "XObject").dict()
                        && let Some(entry) = xobjects.get(name.as_str())
                        && let Obj::Stream(dict, range) = self.doc.resolve(entry)
                        && self.doc.get(dict, "Subtype").name() == Some("Form")
                        && let Some(data) = self.doc.decode(dict, range.clone())
                    {
                        let matrix = match self.doc.get(dict, "Matrix") {
                            Obj::Array(m) => Matrix::from(&m.iter().map(|o| self.doc.resolve(o).clone()).collect::<Vec<_>>()),
                            _ => None,
                        }
                        .unwrap_or(Matrix::IDENTITY);
                        let own = self.doc.get(dict, "Resources").dict().cloned();
                        let res = own.as_ref().or(resources).cloned();
                        self.run(&data, res.as_ref(), matrix.times(state.ctm), depth + 1);
                    }
                }
                _ => {}
            }
            stack.clear();
        }
    }
}

/// The text of the PDF in `data`, page by page, stopping a little after `max` characters.
pub fn text(data: &[u8], max: usize) -> Result<String, OfficeError> {
    let doc = Document::parse(data)?;
    let pages = doc.pages();
    if pages.is_empty() {
        return Err(OfficeError::new("No encontré páginas en el PDF (¿está dañado?)."));
    }
    let mut painter = Painter { doc: &doc, fonts: HashMap::new(), out: String::new(), max, last: None, unmapped: false };
    let mut text = String::new();
    for (n, (page, resources)) in pages.iter().enumerate() {
        let content = match doc.get(page, "Contents") {
            Obj::Array(parts) => parts.iter().filter_map(|p| doc.stream_bytes(p)).collect::<Vec<_>>().join(&b'\n'),
            Obj::Stream(..) => doc.stream_bytes(page.get("Contents").unwrap_or(&Obj::Null)).unwrap_or_default(),
            _ => Vec::new(),
        };
        painter.out.clear();
        painter.last = None;
        painter.run(&content, resources.as_ref(), Matrix::IDENTITY, 0);
        let page_text = painter.out.trim();
        if !page_text.is_empty() {
            text.push_str(&format!("## Página {}\n{page_text}\n\n", n + 1));
        }
        if text.chars().count() > max {
            break;
        }
    }
    if text.trim().is_empty() {
        let why = if painter.unmapped {
            "sus fuentes no traen tabla Unicode, así que su texto no se puede extraer"
        } else {
            "probablemente es un escaneo o solo tiene imágenes"
        };
        return Ok(format!("(El PDF tiene {} páginas pero no tiene capa de texto: {why}. Si el modelo puede ver PDFs o imágenes, que lo lea así.)", pages.len()));
    }
    Ok(text)
}

/// A small but real PDF for the tests: two pages, one with a WinAnsi font, one with a Type0 font and a ToUnicode
/// map, its contents deflated.
#[cfg(test)]
pub fn test_pdf() -> Vec<u8> {
    let page1 = b"BT /F1 12 Tf 72 720 Td (Hola mundo, a\\361o \\(2026\\)) Tj 0 -14 Td [(Segunda) -300 (l\\355nea)] TJ ET".to_vec();
    let page2 = b"q 1 0 0 1 0 0 cm BT /F2 10 Tf 1 0 0 1 72 700 Tm <00010002> Tj 1 0 0 1 72 680 Tm <0003> Tj ET Q".to_vec();
    let deflated = miniz_oxide::deflate::compress_to_vec_zlib(&page2, 6);
    let cmap = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap 1 begincodespacerange <0000> <FFFF> endcodespacerange 2 beginbfchar <0001> <00C9> <0002> <0078> endbfchar 1 beginbfrange <0003> <0003> <00690074006F> endbfrange endcmap end end".to_vec();
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /Resources << /Font << /F1 5 0 R /F2 6 0 R >> >> >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 7 0 R >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents [8 0 R] >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
        b"<< /Type /Font /Subtype /Type0 /BaseFont /X /Encoding /Identity-H /ToUnicode 9 0 R /DescendantFonts [<< /DW 500 >>] >>".to_vec(),
    ];
    let stream = |dict: &str, body: &[u8]| {
        let mut s = format!("<< {dict} /Length {} >>\nstream\n", body.len()).into_bytes();
        s.extend_from_slice(body);
        s.extend_from_slice(b"\nendstream");
        s
    };
    objects.push(stream("", &page1));
    objects.push(stream("/Filter /FlateDecode", &deflated));
    objects.push(stream("", &cmap));
    let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lexer_reads_pdf_objects() {
        let mut lx = Lexer::new(b"<< /A 1 0 R /B [1 2.5 (x\\(y\\)) <414> /N#20m] /C << /D true >> /E null >>", 0);
        let Obj::Dict(d) = lx.object(0).unwrap() else { panic!() };
        assert_eq!(d["A"], Obj::Ref(1));
        assert_eq!(
            d["B"],
            Obj::Array(vec![Obj::Num(1.0), Obj::Num(2.5), Obj::Str(b"x(y)".to_vec()), Obj::Str(vec![0x41, 0x40]), Obj::Name("N m".into())])
        );
        assert_eq!(d["E"], Obj::Null);
        assert_eq!(Lexer::new(b"(a\\101\\nb)", 0).object(0), Some(Obj::Str(b"aA\nb".to_vec())));
        assert_eq!(Lexer::new(b"1 0 0 1 72 700 Tm", 0).object(0), Some(Obj::Num(1.0)), "numbers are not references");
        assert_eq!(ascii85(b"<~87cURD]i,\"Ebo80~>").unwrap(), b"Hello World!");
        assert_eq!(MAC_ROMAN.iter().map(|r| r.chars().count()).sum::<usize>(), 128);
        assert_eq!(mac_roman(0x96), 'ñ');
        let deep = "[".repeat(10_000);
        // Nesting is capped: this returns (whatever it makes of it) instead of overflowing the stack.
        let _ = Lexer::new(deep.as_bytes(), 0).object(0);
        let _ = Lexer::new(format!("{}1", "<<".repeat(10_000)).as_bytes(), 0).object(0);
    }

    #[test]
    fn text_comes_out_page_by_page() {
        let text = text(&test_pdf(), 10_000).unwrap();
        assert!(text.contains("## Página 1\nHola mundo, año (2026)\nSegunda línea"), "{text}");
        assert!(text.contains("## Página 2\nÉx\nito"), "{text}");
    }

    #[test]
    fn broken_encrypted_and_empty_pdfs_say_so() {
        assert!(text(b"no soy un pdf", 100).is_err());
        let mut encrypted = test_pdf();
        let at = find(&encrypted, b"/Root 1 0 R", 0).unwrap();
        encrypted.splice(at..at, b"/Encrypt 99 0 R ".iter().copied());
        assert!(text(&encrypted, 100).unwrap_err().0.contains("cifrado"));
        let blank = String::from_utf8_lossy(&test_pdf()).replace("Tj", "Tx").replace("TJ", "Tx").into_bytes();
        let note = text(&blank, 100).unwrap();
        assert!(note.contains("no tiene capa de texto"), "{note}");
        // Truncated in the middle: what is there is still read.
        let full = test_pdf();
        assert!(text(&full[..full.len() * 2 / 3], 1000).is_ok());
    }
}
