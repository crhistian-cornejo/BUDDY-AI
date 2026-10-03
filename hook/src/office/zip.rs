// Ported from MIKA (MIT, revision d050bc5): apps/macos/Sources/Providers/OfficeFiles.swift (Zip)
//! The ZIP around an Office file, both ways. Writing: XML entries are deflated (miniz_oxide, pure Rust) when that
//! makes them smaller, pictures are stored as they are. Reading: the central directory, stored and deflated entries,
//! every CRC checked and every size capped, so a crafted archive (a "zip bomb") cannot eat the memory.

use super::OfficeError;
use super::xml::Part;

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 == 1 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = crc_table();

/// The standard CRC-32 (IEEE) ZIP stores for every entry.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc = CRC_TABLE[((crc ^ byte as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

/// 2026-01-01 00:00 in MS-DOS form, for every entry: the same input is always the same file.
const DOS_DATE: u16 = ((2026 - 1980) << 9) | (1 << 5) | 1;
/// Bit 11: the names are UTF-8.
const UTF8_NAMES: u16 = 0x0800;
const STORED: u16 = 0;
const DEFLATED: u16 = 8;

fn u16le(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn u32le(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Pictures are compressed already; deflating them again only costs time.
fn worth_deflating(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    !(lower.ends_with(".png") || lower.ends_with(".jpeg") || lower.ends_with(".jpg"))
}

/// A ZIP of `entries`, in order. Fails only past the classic ZIP limits (4 GB, 65 535 entries), which the tools'
/// caps keep far away.
pub fn archive(entries: &[Part]) -> Result<Vec<u8>, OfficeError> {
    let too_big = || OfficeError::new("El archivo resultante es demasiado grande.");
    let count = u16::try_from(entries.len()).map_err(|_| too_big())?;
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (path, data) in entries {
        let name = path.as_bytes();
        let name_len = u16::try_from(name.len()).map_err(|_| too_big())?;
        let size = u32::try_from(data.len()).map_err(|_| too_big())?;
        let offset = u32::try_from(out.len()).map_err(|_| too_big())?;
        let crc = crc32(data);
        let deflated = if worth_deflating(path) && data.len() > 64 {
            Some(miniz_oxide::deflate::compress_to_vec(data, 6)).filter(|d| d.len() < data.len())
        } else {
            None
        };
        let (method, body) = match &deflated {
            Some(d) => (DEFLATED, d.as_slice()),
            None => (STORED, data.as_slice()),
        };
        let packed = u32::try_from(body.len()).map_err(|_| too_big())?;

        u32le(&mut out, 0x0403_4B50);
        u16le(&mut out, 20); // version needed
        u16le(&mut out, UTF8_NAMES);
        u16le(&mut out, method);
        u16le(&mut out, 0); // time
        u16le(&mut out, DOS_DATE);
        u32le(&mut out, crc);
        u32le(&mut out, packed);
        u32le(&mut out, size);
        u16le(&mut out, name_len);
        u16le(&mut out, 0); // extra
        out.extend_from_slice(name);
        out.extend_from_slice(body);

        u32le(&mut central, 0x0201_4B50);
        u16le(&mut central, 20); // made by
        u16le(&mut central, 20); // version needed
        u16le(&mut central, UTF8_NAMES);
        u16le(&mut central, method);
        u16le(&mut central, 0);
        u16le(&mut central, DOS_DATE);
        u32le(&mut central, crc);
        u32le(&mut central, packed);
        u32le(&mut central, size);
        u16le(&mut central, name_len);
        u16le(&mut central, 0); // extra
        u16le(&mut central, 0); // comment
        u16le(&mut central, 0); // disk
        u16le(&mut central, 0); // internal attributes
        u32le(&mut central, 0); // external attributes
        u32le(&mut central, offset);
        central.extend_from_slice(name);
    }
    let central_offset = u32::try_from(out.len()).map_err(|_| too_big())?;
    let central_size = u32::try_from(central.len()).map_err(|_| too_big())?;
    out.extend_from_slice(&central);
    u32le(&mut out, 0x0605_4B50);
    u16le(&mut out, 0);
    u16le(&mut out, 0);
    u16le(&mut out, count);
    u16le(&mut out, count);
    u32le(&mut out, central_size);
    u32le(&mut out, central_offset);
    u16le(&mut out, 0);
    if out.len() > u32::MAX as usize {
        return Err(too_big());
    }
    Ok(out)
}

// MARK: reading

/// Most entries a ZIP we read may list.
const MAX_ENTRIES: usize = 20_000;

/// One file listed in the central directory.
#[derive(Clone, Debug)]
struct Entry {
    name: String,
    method: u16,
    crc: u32,
    packed: usize,
    size: usize,
    local: usize,
}

/// A ZIP held in memory, read through its central directory.
pub struct Archive<'a> {
    data: &'a [u8],
    entries: Vec<Entry>,
}

fn damaged() -> OfficeError {
    OfficeError::new("El archivo está dañado o no es un documento de Office (ZIP no válido).")
}

impl<'a> Archive<'a> {
    pub fn open(data: &'a [u8]) -> Result<Self, OfficeError> {
        let u16at = |o: usize| -> Option<usize> { Some(u16::from_le_bytes(data.get(o..o + 2)?.try_into().ok()?) as usize) };
        let u32at = |o: usize| -> Option<u32> { Some(u32::from_le_bytes(data.get(o..o + 4)?.try_into().ok()?)) };
        // The end record sits in the last 22 bytes plus at most a 65 535-byte comment.
        let last = data.len().checked_sub(22).ok_or_else(damaged)?;
        let first = last.saturating_sub(65_535);
        let end = (first..=last).rev().find(|&o| u32at(o) == Some(0x0605_4B50)).ok_or_else(damaged)?;
        let count = u16at(end + 10).ok_or_else(damaged)?;
        let directory = u32at(end + 16).ok_or_else(damaged)?;
        if count == 0xFFFF || directory == 0xFFFF_FFFF {
            return Err(OfficeError::new("El archivo usa ZIP64, que no sé leer."));
        }
        if count > MAX_ENTRIES {
            return Err(OfficeError::new("El archivo tiene demasiadas partes."));
        }
        let mut offset = directory as usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            if u32at(offset) != Some(0x0201_4B50) {
                return Err(damaged());
            }
            let field = |o: usize| u16at(offset + o).ok_or_else(damaged);
            let long = |o: usize| u32at(offset + o).ok_or_else(damaged);
            let (name_len, extra, comment) = (field(28)?, field(30)?, field(32)?);
            let name = data.get(offset + 46..offset + 46 + name_len).ok_or_else(damaged)?;
            entries.push(Entry {
                name: String::from_utf8_lossy(name).replace('\\', "/"),
                method: field(10)? as u16,
                crc: long(16)?,
                packed: long(20)? as usize,
                size: long(24)? as usize,
                local: long(42)? as usize,
            });
            offset += 46 + name_len + extra + comment;
        }
        Ok(Archive { data, entries })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.iter().any(|e| e.name == name)
    }

    /// The bytes of `name` (no leading slash), at most `limit` of them once unpacked.
    pub fn read(&self, name: &str, limit: usize) -> Result<Vec<u8>, OfficeError> {
        let name = name.trim_start_matches('/');
        let entry = self
            .entries
            .iter()
            .find(|e| e.name == name)
            .ok_or_else(|| OfficeError(format!("Falta la parte {name} dentro del archivo.")))?;
        if entry.size > limit {
            return Err(OfficeError::new("Una parte del archivo es demasiado grande para leerla."));
        }
        let u16at = |o: usize| -> Option<usize> {
            Some(u16::from_le_bytes(self.data.get(o..o + 2)?.try_into().ok()?) as usize)
        };
        if self.data.get(entry.local..entry.local + 4) != Some(&[0x50, 0x4B, 0x03, 0x04]) {
            return Err(damaged());
        }
        let start = entry.local + 30 + u16at(entry.local + 26).ok_or_else(damaged)? + u16at(entry.local + 28).ok_or_else(damaged)?;
        let packed = self.data.get(start..start.checked_add(entry.packed).ok_or_else(damaged)?).ok_or_else(damaged)?;
        let data = match entry.method {
            STORED => packed.to_vec(),
            DEFLATED => miniz_oxide::inflate::decompress_to_vec_with_limit(packed, limit).map_err(|_| damaged())?,
            _ => return Err(OfficeError::new("El archivo usa una compresión que no sé leer.")),
        };
        if data.len() != entry.size || crc32(&data) != entry.crc {
            return Err(damaged());
        }
        Ok(data)
    }
}

/// Every entry of `zip`, unpacked: for the tests. `None` when anything is off.
#[cfg(test)]
pub fn read(zip: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
    let archive = Archive::open(zip).ok()?;
    let names: Vec<String> = archive.names().map(str::to_string).collect();
    names.into_iter().map(|n| archive.read(&n, 1 << 30).ok().map(|d| (n, d))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zip_is_well_formed() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926, "the standard CRC-32 check value");
        let entries = vec![
            ("a.txt".to_string(), b"hola".to_vec()),
            ("dir/b.xml".to_string(), "<x>repetido</x>".repeat(200).into_bytes()),
            ("media/i.png".to_string(), vec![7u8; 500]),
        ];
        let zip = archive(&entries).unwrap();
        let back = read(&zip).unwrap();
        assert_eq!(back, entries);
        assert!(zip.len() < 2000, "the XML is deflated");
        assert_eq!(archive(&entries).unwrap(), zip, "the same input is the same file");
        assert_eq!(read(&archive(&[]).unwrap()).unwrap(), vec![], "an empty archive is still an archive");
    }

    #[test]
    fn a_damaged_zip_is_caught() {
        let mut zip = archive(&[("a.txt".to_string(), b"hola".to_vec())]).unwrap();
        zip[30 + 5] ^= 0xFF; // first byte of the data, after the 30-byte header and the 5-byte name
        assert!(read(&zip).is_none());
        assert!(Archive::open(b"no soy un zip").is_err());
        assert!(Archive::open(&[]).is_err());
    }

    #[test]
    fn an_entry_bigger_than_the_limit_is_refused() {
        let zip = archive(&[("big.xml".to_string(), vec![b'a'; 100_000])]).unwrap();
        assert!(zip.len() < 2_000, "a bomb in miniature");
        let archive = Archive::open(&zip).unwrap();
        assert!(archive.read("big.xml", 1000).is_err());
        assert_eq!(archive.read("big.xml", 100_000).unwrap().len(), 100_000);
        assert!(archive.read("missing.xml", 10).is_err());
    }
}
