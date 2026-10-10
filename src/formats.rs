//! Explicit extension dispatch shared with folder eligibility (no global hooks).
use std::{io::Cursor, path::Path};

use image::{ImageDecoder, ImageFormat, ImageReader, Limits};

use crate::decode::{LoadError, MAX_DIMENSION};

#[derive(Clone, Copy, Debug)]
pub enum Format {
    Standard(ImageFormat),
    Cur,
    Icns,
    Wbmp,
    Xbm,
    Xpm,
    Pcx,
    Sgi,
    Jxl,
}

pub fn format(path: &Path) -> Option<Format> {
    let ext = path.extension()?.as_encoded_bytes();
    Some(match ext.to_ascii_lowercase().as_slice() {
        b"jpg" | b"jpeg" => Format::Standard(ImageFormat::Jpeg),
        b"png" => Format::Standard(ImageFormat::Png),
        b"tif" | b"tiff" => Format::Standard(ImageFormat::Tiff),
        b"bmp" => Format::Standard(ImageFormat::Bmp),
        b"ico" => Format::Standard(ImageFormat::Ico),
        b"cur" => Format::Cur,
        b"tga" => Format::Standard(ImageFormat::Tga),
        b"pnm" | b"pbm" | b"pgm" | b"ppm" | b"pam" => Format::Standard(ImageFormat::Pnm),
        b"gif" => Format::Standard(ImageFormat::Gif),
        b"webp" => Format::Standard(ImageFormat::WebP),
        b"icns" => Format::Icns,
        b"wbmp" => Format::Wbmp,
        b"xbm" => Format::Xbm,
        b"xpm" => Format::Xpm,
        b"pcx" => Format::Pcx,
        b"jxl" => Format::Jxl,
        b"sgi" | b"bw" | b"rgb" | b"rgba" => Format::Sgi,
        _ => return None,
    })
}

pub fn decoder(
    format: Format,
    mut encoded: Vec<u8>,
    limits: Limits,
) -> Result<Box<dyn ImageDecoder>, LoadError> {
    if matches!(format, Format::Cur) {
        encoded = cursor_icon(&encoded)?;
    }
    let cursor = Cursor::new(encoded);
    let mut decoder: Box<dyn ImageDecoder> = match format {
        Format::Standard(format) => {
            let mut reader = ImageReader::with_format(cursor, format);
            reader.limits(limits.clone());
            Box::new(reader.into_decoder()?)
        }
        Format::Cur => Box::new(image::codecs::ico::IcoDecoder::new(cursor)?),
        Format::Icns => Box::new(image_extras::icns::IcnsDecoder::new(cursor)?),
        Format::Wbmp => Box::new(image_extras::wbmp::WbmpDecoder::new(cursor)?),
        Format::Xbm => Box::new(image_extras::xbm::XbmDecoder::new(cursor)?),
        Format::Xpm => Box::new(image_extras::xpm::XpmDecoder::new(cursor)?),
        Format::Pcx => Box::new(image_extras::pcx::PCXDecoder::new(cursor)?),
        Format::Jxl => {
            return Err(LoadError::UnsupportedVariant(
                "JXL: use core decoder".into(),
            ));
        }
        Format::Sgi => Box::new(image_extras::sgi::SgiDecoder::new(cursor)?),
    };
    decoder.set_limits(limits)?;
    Ok(decoder)
}

// CUR entries use the same PNG/DIB payloads as ICO, but the two u16 fields
// are hotspots, not planes/depth. Validate the directory, select largest area
// (first wins ties), then wrap that payload in a one-entry ICO with neutral
// planes/depth. Hotspots do not affect a static image preview.
fn cursor_icon(bytes: &[u8]) -> Result<Vec<u8>, LoadError> {
    let malformed = || LoadError::Malformed("CUR: invalid or truncated icon directory".into());
    if bytes.get(..4) != Some(&[0, 0, 2, 0]) {
        return Err(malformed());
    }
    let count = bytes.get(4..6).ok_or_else(malformed)?;
    let count = usize::from(u16::from_le_bytes([count[0], count[1]]));
    let directory_end = 6 + count * 16;
    if count == 0 || directory_end > bytes.len() {
        return Err(malformed());
    }
    let mut selected = None;
    let mut area = 0;
    for entry in bytes[6..directory_end].chunks_exact(16) {
        let width = if entry[0] == 0 {
            256
        } else {
            u32::from(entry[0])
        };
        let height = if entry[1] == 0 {
            256
        } else {
            u32::from(entry[1])
        };
        let length = u32::from_le_bytes(entry[8..12].try_into().expect("directory entry")) as usize;
        let offset =
            u32::from_le_bytes(entry[12..16].try_into().expect("directory entry")) as usize;
        let end = offset.checked_add(length).ok_or_else(malformed)?;
        if entry[3] != 0 || length == 0 || offset < directory_end || end > bytes.len() {
            return Err(malformed());
        }
        if width <= MAX_DIMENSION && height <= MAX_DIMENSION && width * height > area {
            area = width * height;
            selected = Some((entry, &bytes[offset..end]));
        }
    }
    let (entry, payload) = selected.ok_or_else(malformed)?;
    let mut icon = vec![0, 0, 1, 0, 1, 0];
    icon.extend_from_slice(entry);
    icon[10..14].copy_from_slice(&[1, 0, 0, 0]);
    icon[18..22].copy_from_slice(&22u32.to_le_bytes());
    icon.extend_from_slice(payload);
    Ok(icon)
}
