//! Stored document composites only; never extract archives or render layers.
use std::io::{Cursor, Read};

use super::{LoadError, MAX_DIMENSION, MAX_ENCODED_BYTES, Pixels};
use crate::formats::Format;

const MAX_ENTRIES: usize = 4096;
fn malformed(kind: &str, reason: impl std::fmt::Display) -> LoadError {
    LoadError::Malformed(format!("{kind}: {reason}"))
}
fn unsupported(reason: impl Into<String>) -> LoadError {
    LoadError::UnsupportedVariant(reason.into())
}

pub(super) fn archive(format: Format, bytes: &[u8]) -> Result<Pixels, LoadError> {
    let kind = if matches!(format, Format::Kra) {
        "KRA"
    } else {
        "ORA"
    };
    // Bound central-directory allocations before the library opens it. Ordinary
    // documents use classic ZIP; ZIP64 is unnecessary inside our 64 MiB input cap.
    let end = bytes.len().saturating_sub(22);
    let eocd = (bytes.len().saturating_sub(65557)..=end)
        .rev()
        .find(|&p| {
            bytes.get(p..p + 4) == Some(b"PK\x05\x06")
                && bytes.get(p + 20..p + 22).is_some_and(|n| {
                    p + 22 + usize::from(u16::from_le_bytes([n[0], n[1]])) == bytes.len()
                })
        })
        .ok_or_else(|| malformed(kind, "missing ZIP end directory"))?;
    if eocd >= 20 && bytes.get(eocd - 20..eocd - 16) == Some(b"PK\x06\x07") {
        return Err(unsupported(format!("{kind}: ZIP64 archive")));
    }
    let count = u16::from_le_bytes([bytes[eocd + 10], bytes[eocd + 11]]) as usize;
    if count > MAX_ENTRIES {
        return Err(LoadError::ResourceLimit(format!(
            "{kind}: ZIP entry count exceeds {MAX_ENTRIES}"
        )));
    }
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| malformed(kind, e))?;
    if zip.len() > MAX_ENTRIES {
        return Err(LoadError::ResourceLimit(format!(
            "{kind}: ZIP entry count exceeds {MAX_ENTRIES}"
        )));
    }
    if zip.len() != count {
        return Err(malformed(
            kind,
            "ZIP directory count mismatch or duplicate entry names",
        ));
    }
    let mut selected = None;
    for i in 0..zip.len() {
        let entry = zip.by_index_raw(i).map_err(|e| malformed(kind, e))?;
        let name = entry.name().map_err(|e| malformed(kind, e))?;
        if name.starts_with('/')
            || name.contains('\\')
            || name.contains(':')
            || name.split('/').any(|part| part == "..")
            || name.contains('\0')
        {
            return Err(malformed(kind, "unsafe ZIP entry name"));
        }
        // Both Krita and OpenRaster store the full merged preview at this exact
        // root name. preview.png is a thumbnail and is deliberately not a fallback.
        if name == "mergedimage.png" {
            if selected.replace(i).is_some() {
                return Err(malformed(kind, "duplicate mergedimage.png"));
            }
            if entry.size() > MAX_ENCODED_BYTES {
                return Err(LoadError::ResourceLimit(format!(
                    "{kind}: merged PNG exceeds 64 MiB"
                )));
            }
        }
    }
    let i = selected.ok_or_else(|| malformed(kind, "missing mergedimage.png"))?;
    let mut entry = zip.by_index(i).map_err(|e| malformed(kind, e))?;
    let declared = entry.size();
    let mut png = Vec::new();
    entry
        .by_ref()
        .take(MAX_ENCODED_BYTES + 1)
        .read_to_end(&mut png)
        .map_err(|e| malformed(kind, format!("mergedimage.png: {e}")))?;
    if png.len() as u64 > MAX_ENCODED_BYTES {
        return Err(LoadError::ResourceLimit(format!(
            "{kind}: expanded merged PNG exceeds 64 MiB"
        )));
    }
    if png.len() as u64 != declared {
        return Err(malformed(kind, "mergedimage.png size mismatch"));
    }
    // Reuse ordinary PNG metadata, ICC, high-depth, orientation and level paths.
    super::decode_encoded(Format::Standard(image::ImageFormat::Png), png).map_err(|e| match e {
        LoadError::Malformed(reason) => malformed(kind, format!("mergedimage.png: {reason}")),
        other => other,
    })
}

fn u16be(bytes: &[u8], p: usize) -> Result<u16, LoadError> {
    let b = bytes
        .get(p..p + 2)
        .ok_or_else(|| malformed("PSD", "truncated field"))?;
    Ok(u16::from_be_bytes([b[0], b[1]]))
}
fn u32be(bytes: &[u8], p: usize) -> Result<u32, LoadError> {
    let b = bytes
        .get(p..p + 4)
        .ok_or_else(|| malformed("PSD", "truncated field"))?;
    Ok(u32::from_be_bytes(b.try_into().expect("four bytes")))
}
fn section<'a>(bytes: &'a [u8], p: &mut usize) -> Result<&'a [u8], LoadError> {
    let len = u32be(bytes, *p)? as usize;
    *p += 4;
    let end = p
        .checked_add(len)
        .ok_or_else(|| malformed("PSD", "section overflow"))?;
    let data = bytes
        .get(*p..end)
        .ok_or_else(|| malformed("PSD", "truncated section"))?;
    *p = end;
    Ok(data)
}

pub(super) fn psd(bytes: &[u8]) -> Result<Pixels, LoadError> {
    if bytes.get(..4) != Some(b"8BPS") {
        return Err(malformed("PSD", "invalid signature"));
    }
    match u16be(bytes, 4)? {
        1 => (),
        2 => return Err(unsupported("PSD: PSB large document")),
        _ => return Err(malformed("PSD", "invalid version")),
    }
    let depth = u16be(bytes, 22)?;
    if depth != 8 {
        return Err(unsupported(format!(
            "PSD: {depth}-bit samples (only 8-bit supported)"
        )));
    }
    let mode = u16be(bytes, 24)?;
    match mode {
        1 | 3 => (),
        4 => return Err(unsupported("PSD: CMYK color mode")),
        9 => return Err(unsupported("PSD: Lab color mode")),
        _ => return Err(unsupported(format!("PSD: color mode {mode}"))),
    }
    let channels = usize::from(u16be(bytes, 12)?);
    if !(if mode == 1 { 1..=2 } else { 3..=4 }).contains(&channels) {
        return Err(unsupported(format!(
            "PSD: {channels} channels for color mode {mode}"
        )));
    }
    let height = u32be(bytes, 14)?;
    let width = u32be(bytes, 18)?;
    let size = super::checked_size(width, height, MAX_DIMENSION)?;
    let plane = size[0] * size[1];
    let expected = plane * channels;
    let mut p = 26;
    section(bytes, &mut p)?; // color mode data is not needed for RGB/grayscale
    let resources = section(bytes, &mut p)?;
    section(bytes, &mut p)?; // skip layers entirely, including their allocations
    let compression = u16be(bytes, p)?;
    p += 2;
    let data = bytes
        .get(p..)
        .ok_or_else(|| malformed("PSD", "missing composite"))?;
    let raw = match compression {
        0 => {
            if data.len() != expected {
                return Err(malformed("PSD", "raw composite size mismatch"));
            }
            data.to_vec()
        }
        1 => unpack_rows(data, size[0], size[1] * channels)?,
        2 => {
            // psd 0.3.5 panics on ZIP. Adapt using the existing bounded zlib seam,
            // then supply raw planes to its merged-composite API.
            let mut raw = Vec::new();
            flate2::read::ZlibDecoder::new(data)
                .take(expected as u64 + 1)
                .read_to_end(&mut raw)
                .map_err(|e| malformed("PSD", e))?;
            if raw.len() != expected {
                return Err(malformed("PSD", "ZIP composite size mismatch"));
            }
            raw
        }
        3 => return Err(unsupported("PSD: ZIP-prediction compression")),
        _ => return Err(malformed("PSD", "invalid composite compression")),
    };
    // Build a bounded composite-only PSD. Removing unused layers/resources avoids
    // upstream eager layer parsing; ICC is independently read below.
    let mut composite = bytes[..26].to_vec();
    if mode == 1 {
        composite[12..14].copy_from_slice(&1u16.to_be_bytes());
    }
    composite.extend([0; 14]); // three zero section lengths + raw compression
    composite.extend_from_slice(if mode == 1 { &raw[..plane] } else { &raw });
    let doc = psd::Psd::from_bytes(&composite).map_err(|e| malformed("PSD", e))?;
    let mut rgba = doc.rgba();
    if mode == 1 && channels == 2 {
        for (pixel, alpha) in rgba.chunks_exact_mut(4).zip(&raw[plane..]) {
            pixel[3] = *alpha;
        }
    }
    let icc = icc_resource(resources)?;
    let warning = crate::color::normalize8(&mut rgba, icc, false);
    let levels = crate::levels::generate(size, &rgba)?;
    Ok(Pixels {
        size,
        rgba,
        levels,
        warning,
        ..Default::default()
    })
}

fn unpack_rows(data: &[u8], width: usize, rows: usize) -> Result<Vec<u8>, LoadError> {
    let table = data
        .get(..rows * 2)
        .ok_or_else(|| malformed("PSD", "truncated RLE row table"))?;
    let mut p = rows * 2;
    let mut raw = Vec::new();
    for n in table.chunks_exact(2) {
        let len = usize::from(u16::from_be_bytes([n[0], n[1]]));
        let row = data
            .get(p..p + len)
            .ok_or_else(|| malformed("PSD", "truncated RLE row"))?;
        p += len;
        let start = raw.len();
        let mut r = 0;
        while r < row.len() {
            let tag = row[r] as i8;
            r += 1;
            if tag == -128 {
                continue;
            }
            let count = if tag >= 0 {
                tag as usize + 1
            } else {
                (1i16 - i16::from(tag)) as usize
            };
            if raw.len() - start + count > width {
                return Err(malformed("PSD", "RLE row exceeds width"));
            }
            if tag >= 0 {
                let literal = row
                    .get(r..r + count)
                    .ok_or_else(|| malformed("PSD", "truncated RLE literal"))?;
                raw.extend_from_slice(literal);
                r += count;
            } else {
                let value = *row
                    .get(r)
                    .ok_or_else(|| malformed("PSD", "truncated RLE repeat"))?;
                raw.resize(raw.len() + count, value);
                r += 1;
            }
        }
        if raw.len() - start != width {
            return Err(malformed("PSD", "RLE row width mismatch"));
        }
    }
    if p != data.len() {
        return Err(malformed("PSD", "trailing RLE bytes"));
    }
    Ok(raw)
}

fn icc_resource(mut data: &[u8]) -> Result<Option<&[u8]>, LoadError> {
    let mut icc = None;
    while !data.is_empty() {
        if data.get(..4) != Some(b"8BIM") {
            return Err(malformed("PSD", "invalid image resource signature"));
        }
        let id = u16be(data, 4)?;
        let name_len = usize::from(
            *data
                .get(6)
                .ok_or_else(|| malformed("PSD", "truncated resource name"))?,
        );
        let p = 6 + (name_len + 2) / 2 * 2;
        let len = u32be(data, p)? as usize;
        let end = p + 4 + len;
        let payload = data
            .get(p + 4..end)
            .ok_or_else(|| malformed("PSD", "truncated resource"))?;
        if id == 1039 {
            icc = Some(payload);
        }
        data = data
            .get(end + len % 2..)
            .ok_or_else(|| malformed("PSD", "truncated resource padding"))?;
    }
    Ok(icc)
}

#[cfg(test)]
mod tests;
