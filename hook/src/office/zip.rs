// Ported from MIKA (MIT, revision d050bc5): apps/macos/Sources/Providers/OfficeFiles.swift (Zip)
//! The ZIP around an Office file. Every entry is stored as is (no compression), so no library is involved: Word,
//! Excel and PowerPoint open a stored archive exactly like a deflated one.

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

fn u16le(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn u32le(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// A ZIP of `entries`, in order, each stored as is. Fails only past the classic ZIP limits (4 GB, 65 535 entries),
/// which the tools' caps keep far away.
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

        u32le(&mut out, 0x0403_4B50);
        u16le(&mut out, 20); // version needed
        u16le(&mut out, UTF8_NAMES);
        u16le(&mut out, 0); // stored
        u16le(&mut out, 0); // time
        u16le(&mut out, DOS_DATE);
        u32le(&mut out, crc);
        u32le(&mut out, size);
        u32le(&mut out, size);
        u16le(&mut out, name_len);
        u16le(&mut out, 0); // extra
        out.extend_from_slice(name);
        out.extend_from_slice(data);

        u32le(&mut central, 0x0201_4B50);
        u16le(&mut central, 20); // made by
        u16le(&mut central, 20); // version needed
        u16le(&mut central, UTF8_NAMES);
        u16le(&mut central, 0);
        u16le(&mut central, 0);
        u16le(&mut central, DOS_DATE);
        u32le(&mut central, crc);
        u32le(&mut central, size);
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

/// Reads a stored ZIP back through its central directory, checking every CRC: an independent little parser for
/// the tests. `None` when anything is off.
#[cfg(test)]
pub fn read(zip: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
    let u16at = |o: usize| -> Option<usize> { Some(u16::from_le_bytes(zip.get(o..o + 2)?.try_into().ok()?) as usize) };
    let u32at = |o: usize| -> Option<u32> { Some(u32::from_le_bytes(zip.get(o..o + 4)?.try_into().ok()?)) };
    let mut end = zip.len().checked_sub(22)?;
    while u32at(end)? != 0x0605_4B50 {
        end = end.checked_sub(1)?;
    }
    let count = u16at(end + 10)?;
    let mut offset = u32at(end + 16)? as usize;
    let mut out = Vec::new();
    for _ in 0..count {
        if u32at(offset)? != 0x0201_4B50 || u16at(offset + 10)? != 0 {
            return None;
        }
        let crc = u32at(offset + 16)?;
        let size = u32at(offset + 20)? as usize;
        let (name_len, extra, comment) = (u16at(offset + 28)?, u16at(offset + 30)?, u16at(offset + 32)?);
        let local = u32at(offset + 42)? as usize;
        let name = String::from_utf8(zip.get(offset + 46..offset + 46 + name_len)?.to_vec()).ok()?;
        if u32at(local)? != 0x0403_4B50 {
            return None;
        }
        let start = local + 30 + u16at(local + 26)? + u16at(local + 28)?;
        let data = zip.get(start..start + size)?.to_vec();
        if crc32(&data) != crc {
            return None;
        }
        out.push((name, data));
        offset += 46 + name_len + extra + comment;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zip_is_well_formed() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926, "the standard CRC-32 check value");
        let entries = vec![("a.txt".to_string(), b"hola".to_vec()), ("dir/b.xml".to_string(), b"<x/>".to_vec())];
        let zip = archive(&entries).unwrap();
        let back = read(&zip).unwrap();
        assert_eq!(back, entries);
        assert_eq!(archive(&entries).unwrap(), zip, "the same input is the same file");
        assert_eq!(read(&archive(&[]).unwrap()).unwrap(), vec![], "an empty archive is still an archive");
    }

    #[test]
    fn a_damaged_zip_is_caught_by_the_test_reader() {
        let mut zip = archive(&[("a.txt".to_string(), b"hola".to_vec())]).unwrap();
        zip[30 + 5] ^= 0xFF; // first byte of the data, after the 30-byte header and the 5-byte name
        assert!(read(&zip).is_none());
    }
}
