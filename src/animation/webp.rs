//! Bounded libwebp adapter. Container inspection never decodes or allocates a canvas.
use crate::decode::LoadError;
use image::{ImageDecoder, RgbaImage};
use std::{borrow::Cow, io::Cursor, time::Duration};

/// Keep the current image-webp compositor behavior: with no explicit background
/// override it retains the previous rectangle even when ANMF requests disposal.
/// Only native input needs normalization; encoded files and fallback input stay
/// untouched. A scoped owner keeps the wrapper's borrowed input alive safely.
pub(super) fn prepare(bytes: &[u8]) -> Result<Cow<'_, [u8]>, LoadError> {
    let decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes))?;
    if !decoder.has_animation() {
        return Ok(Cow::Borrowed(bytes));
    }
    let dimensions = decoder.dimensions();
    super::check_canvas(dimensions)?;
    let preflight = Preflight::read(bytes, dimensions)?;
    if !preflight.use_native(dimensions) || !preflight.has_disposal {
        return Ok(Cow::Borrowed(bytes));
    }
    if bytes.len() as u64 > crate::decode::MAX_ENCODED_BYTES {
        return Err(LoadError::ResourceLimit(
            "Encoded image exceeds 64 MiB".into(),
        ));
    }
    // At the native area cap, 12 RGBA canvases occupy <380 MiB. This <=64 MiB
    // compatibility copy fits the remaining space in the existing 512 MiB reserve.
    let mut compatible = bytes.to_vec();
    let end = u32::from_le_bytes(bytes[4..8].try_into().map_err(|_| malformed())?) as usize + 8;
    let mut at = 12;
    while at < end {
        // Preflight validated these exact chunk boundaries before this allocation.
        let size =
            u32::from_le_bytes(bytes[at + 4..at + 8].try_into().map_err(|_| malformed())?) as usize;
        if &bytes[at..at + 4] == b"ANMF" {
            compatible[at + 8 + 15] &= !1;
        }
        at += 8 + size + (size & 1);
    }
    Ok(Cow::Owned(compatible))
}

const MAX_NATIVE_PIXELS: u64 = 8_294_400;

fn malformed() -> LoadError {
    LoadError::Malformed("WebP: invalid animation metadata or incomplete frame decode".into())
}

pub(super) struct Preflight {
    pub frames: usize,
    duration_ms: u64,
    has_disposal: bool,
}

impl Preflight {
    pub fn read(bytes: &[u8], dimensions: (u32, u32)) -> Result<Self, LoadError> {
        if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WEBP") {
            return Err(malformed());
        }
        let size = u32::from_le_bytes(bytes[4..8].try_into().map_err(|_| malformed())?);
        let end = usize::try_from(u64::from(size) + 8).map_err(|_| malformed())?;
        let container = bytes.get(..end).ok_or_else(malformed)?;
        let mut at = 12usize;
        let mut frames = 0;
        let mut duration_ms = 0u64;
        let mut has_disposal = false;
        let mut canvas = None;
        while at < end {
            let header = container.get(at..at + 8).ok_or_else(malformed)?;
            let size =
                u32::from_le_bytes(header[4..8].try_into().map_err(|_| malformed())?) as usize;
            at += 8;
            let next = at.checked_add(size).ok_or_else(malformed)?;
            let chunk = container.get(at..next).ok_or_else(malformed)?;
            match &header[..4] {
                b"VP8X" => {
                    if chunk.len() != 10 || canvas.is_some() {
                        return Err(malformed());
                    }
                    canvas = Some((u24(&chunk[4..7]) + 1, u24(&chunk[7..10]) + 1));
                }
                b"ANMF" => {
                    if chunk.len() < 16 {
                        return Err(malformed());
                    }
                    frames += 1;
                    duration_ms += u64::from(u24(&chunk[12..15]));
                    has_disposal |= chunk[15] & 1 != 0;
                }
                _ => {}
            }
            at = next.checked_add(size & 1).ok_or_else(malformed)?;
            if at > end {
                return Err(malformed());
            }
        }
        // Check the native canvas header against the resource-checked image decoder
        // before libwebp's constructor allocates either of its canvas buffers.
        if canvas != Some(dimensions) || frames == 0 {
            return Err(malformed());
        }
        Ok(Self {
            frames,
            duration_ms,
            has_disposal,
        })
    }

    pub fn use_native(&self, (width, height): (u32, u32)) -> bool {
        u64::from(width) * u64::from(height) <= MAX_NATIVE_PIXELS
            && self.duration_ms <= i32::MAX as u64
    }
}

fn u24(bytes: &[u8]) -> u32 {
    u32::from(bytes[0]) | (u32::from(bytes[1]) << 8) | (u32::from(bytes[2]) << 16)
}

pub(super) struct Native<'a> {
    frames: webp_animation::DecoderIterator<'a>,
    remaining: usize,
    timestamp: i32,
    dimensions: (u32, u32),
}

impl<'a> Native<'a> {
    pub fn new(
        bytes: &'a [u8],
        dimensions: (u32, u32),
        preflight: Preflight,
    ) -> Result<Self, LoadError> {
        let decoder = webp_animation::Decoder::new(bytes)
            .map_err(|e| LoadError::Malformed(format!("WebP: {e:?}")))?;
        if decoder.dimensions() != dimensions {
            return Err(malformed());
        }
        Ok(Self {
            frames: decoder.into_iter(),
            remaining: preflight.frames,
            timestamp: 0,
            dimensions,
        })
    }

    pub fn next(&mut self) -> Result<Option<(RgbaImage, Duration)>, LoadError> {
        let frame = self.frames.next();
        // The wrapper returns None both at EOF and on native decode failure.
        // Only the independently counted number of ANMF chunks establishes EOF.
        if self.remaining == 0 {
            return if frame.is_none() {
                Ok(None)
            } else {
                Err(malformed())
            };
        }
        let frame = frame.ok_or_else(malformed)?;
        let timestamp = frame.timestamp();
        let delay = timestamp
            .checked_sub(self.timestamp)
            .filter(|d| *d >= 0)
            .ok_or_else(malformed)?;
        if frame.dimensions() != self.dimensions {
            return Err(malformed());
        }
        // The wrapper exposes a slice, not an owned-buffer extraction API. This
        // transient copy fits the existing compositor reserve; the wrapper frame
        // is dropped before orientation, color normalization and level generation.
        let rgba = RgbaImage::from_raw(self.dimensions.0, self.dimensions.1, frame.data().to_vec())
            .ok_or_else(malformed)?;
        self.remaining -= 1;
        self.timestamp = timestamp;
        Ok(Some((rgba, Duration::from_millis(delay as u64))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_boundaries_are_inclusive_and_area_based() {
        let mut p = Preflight {
            frames: 1,
            duration_ms: i32::MAX as u64,
            has_disposal: false,
        };
        assert!(p.use_native((3840, 2160)));
        assert!(p.use_native((7680, 1080)));
        assert!(!p.use_native((3841, 2160)));
        p.duration_ms += 1;
        assert!(!p.use_native((1, 1)));
    }
}
