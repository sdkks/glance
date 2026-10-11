use std::{fmt, fs::File, io::Read, path::Path};

use image::{DynamicImage, ImageError, ImageFormat, Limits};

pub const MAX_ENCODED_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_DECODED_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_DIMENSION: u32 = 16384;

#[derive(Default)]
pub struct Pixels {
    pub svg: Option<std::sync::Arc<svg::Document>>,
    pub svg_scale: f32,
    pub animation_delay: Option<std::time::Duration>,
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
    pub rgba16: Option<Vec<u16>>,
    pub warning: Option<String>,
    pub levels: Vec<crate::levels::Level>,
}

impl Pixels {
    // Inspect the authoritative straight-alpha source, never rounded clipboard pixels.
    pub fn has_transparency(&self) -> bool {
        if let Some(rgba) = &self.rgba16 {
            rgba.chunks_exact(4).any(|pixel| pixel[3] != u16::MAX)
        } else {
            self.rgba.chunks_exact(4).any(|pixel| pixel[3] != u8::MAX)
        }
    }

    pub fn accounted_bytes(&self) -> usize {
        self.source_bytes() + self.levels.iter().map(|l| l.rgba.len()).sum::<usize>()
    }

    pub fn source_bytes(&self) -> usize {
        self.rgba16
            .as_ref()
            .map_or(self.rgba.len(), |p| p.len() * 2)
    }

    pub fn clipboard_pixel(&self, index: usize) -> [u8; 4] {
        if let Some(source) = &self.rgba16 {
            std::array::from_fn(|c| ((u32::from(source[index * 4 + c]) + 128) / 257) as u8)
        } else {
            self.rgba[index * 4..index * 4 + 4]
                .try_into()
                .expect("checked source")
        }
    }
}

#[cfg(test)]
mod transparency_tests {
    use super::*;

    #[test]
    fn rgba_detects_partial_and_fully_transparent_pixels_only() {
        let mut pixels = Pixels {
            rgba: vec![0, 128, 255, 255, 255, 0, 0, 255],
            ..Default::default()
        };
        assert!(!pixels.has_transparency());
        for alpha in [254, 128, 0] {
            pixels.rgba[7] = alpha;
            assert!(pixels.has_transparency());
        }
        assert!(!Pixels::default().has_transparency());
    }

    #[test]
    fn rgba16_checks_full_precision_and_takes_precedence_over_rgba() {
        let mut pixels = Pixels {
            rgba: vec![0; 4],
            rgba16: Some(vec![0, 32768, 65535, 65535, 65535, 0, 0, 65535]),
            ..Default::default()
        };
        assert!(!pixels.has_transparency());
        for alpha in [65534, 32768, 0] {
            pixels.rgba16.as_mut().unwrap()[7] = alpha;
            assert!(pixels.has_transparency());
        }
    }
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Malformed(String),
    Unsupported,
    UnsupportedVariant(String),
    ResourceLimit(String),
    DisplayUnavailable(String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "Could not read file: {error}"),
            Self::Malformed(error) => write!(f, "Could not decode image: {error}"),
            Self::Unsupported => write!(
                f,
                "Unsupported format. See README for supported extensions."
            ),
            Self::UnsupportedVariant(reason) => write!(f, "Unsupported image variant: {reason}"),
            Self::DisplayUnavailable(reason) => write!(f, "Display unavailable: {reason}"),
            Self::ResourceLimit(reason) => {
                write!(f, "Image exceeds this version's resource limit: {reason}")
            }
        }
    }
}

impl From<std::io::Error> for LoadError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ImageError> for LoadError {
    fn from(error: ImageError) -> Self {
        match error {
            ImageError::IoError(error) => Self::Io(error),
            ImageError::Limits(error) => Self::ResourceLimit(error.to_string()),
            ImageError::Unsupported(error) => Self::UnsupportedVariant(error.to_string()),
            error => Self::Malformed(error.to_string()),
        }
    }
}

pub fn load(path: &Path, _texture_limit: u32) -> Result<Pixels, LoadError> {
    let format = crate::formats::format(path).ok_or(LoadError::Unsupported)?;
    let encoded = read_encoded(path)?;
    // image-extras explicitly does not prioritize fuzzing and may panic on bad
    // input (PCX uses unchecked palette indexing, for example). Contain unwinding
    // through construction, metadata and decoding so workers remain usable.
    // This cannot contain allocator aborts or promise memory-safety isolation.
    std::panic::catch_unwind(|| decode_encoded(format, encoded)).unwrap_or_else(|_| {
        Err(LoadError::Malformed(format!(
            "{format:?}: decoder panicked on malformed input"
        )))
    })
}

pub(crate) fn read_encoded(path: &Path) -> Result<Vec<u8>, LoadError> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_ENCODED_BYTES {
        return Err(LoadError::ResourceLimit(
            "encoded input exceeds 64 MiB".into(),
        ));
    }
    let mut encoded = Vec::new();
    file.take(MAX_ENCODED_BYTES + 1).read_to_end(&mut encoded)?;
    if encoded.len() as u64 > MAX_ENCODED_BYTES {
        return Err(LoadError::ResourceLimit(
            "encoded input exceeds 64 MiB".into(),
        ));
    }
    Ok(encoded)
}

fn decode_encoded(format: crate::formats::Format, encoded: Vec<u8>) -> Result<Pixels, LoadError> {
    if matches!(format, crate::formats::Format::Psd) {
        return document::psd(&encoded);
    }
    if matches!(
        format,
        crate::formats::Format::Kra | crate::formats::Format::Ora
    ) {
        return document::archive(format, &encoded);
    }
    if matches!(format, crate::formats::Format::Avif) {
        return avif::decode(&encoded);
    }
    if matches!(
        format,
        crate::formats::Format::Svg | crate::formats::Format::Svgz
    ) {
        return svg::decode(&encoded, matches!(format, crate::formats::Format::Svgz));
    }
    if matches!(format, crate::formats::Format::Jxl) {
        return jxl::decode(&encoded);
    }
    if let Some(frame) = crate::animation::first(format, &encoded)? {
        return Ok(frame);
    }
    let jpeg_cmyk = matches!(format, crate::formats::Format::Standard(ImageFormat::Jpeg))
        && crate::color::jpeg_cmyk(&encoded);
    let dimension_limit = MAX_DIMENSION;
    let mut limits = Limits::default();
    limits.max_image_width = Some(dimension_limit);
    limits.max_image_height = Some(dimension_limit);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    let mut decoder = crate::formats::decoder(format, encoded, limits)?;
    let (width, height) = decoder.dimensions();
    checked_size(width, height, dimension_limit)?;
    if decoder.total_bytes() > MAX_DECODED_BYTES {
        return Err(LoadError::ResourceLimit(format!(
            "decoded samples exceed {} MiB; decoded RGBA8 budget {} MiB, maximum side {dimension_limit}",
            MAX_DECODED_BYTES / (1024 * 1024),
            MAX_DECODED_BYTES / (1024 * 1024)
        )));
    }
    let high_depth = decoder.color_type().bits_per_pixel()
        / u16::from(decoder.color_type().channel_count())
        == 16;
    let float = matches!(
        decoder.color_type(),
        image::ColorType::Rgb32F | image::ColorType::Rgba32F
    );
    let cmyk = jpeg_cmyk
        || matches!(
            decoder.original_color_type(),
            image::ExtendedColorType::Cmyk8 | image::ExtendedColorType::Cmyk16
        );
    if float {
        checked_output_bytes(width, height, 16)?;
    }
    checked_output_bytes(width, height, if high_depth || float { 8 } else { 4 })?;
    // Metadata must be obtained before consuming the decoder. A metadata error
    // is disclosed as a color fallback, not silently treated as untagged.
    let (icc, metadata_warning) = match decoder.icc_profile() {
        Ok(icc) => (icc, None),
        Err(_) => (None, Some(crate::color::FALLBACK.to_owned())),
    };
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    let size = [image.width() as usize, image.height() as usize];
    if float {
        // TIFF exposes float samples but not a verified chromaticity/HDR-transfer
        // contract. Disclose the fixed scene-linear sRGB assumption.
        let rgba16 = crate::color::float_to_sdr(image.into_rgba32f().as_raw())?;
        let levels = crate::levels::generate16(size, &rgba16)?;
        let warning = Some(if icc.is_some() || metadata_warning.is_some() {
            "Float SDR preview: assuming scene-linear sRGB; ICC not applied".into()
        } else {
            "Float SDR preview: assuming scene-linear sRGB, fixed Reinhard white 4".into()
        });
        Ok(Pixels {
            size,
            rgba: vec![],
            rgba16: Some(rgba16),
            levels,
            warning,
            animation_delay: None,
            ..Default::default()
        })
    } else if high_depth {
        let mut rgba16 = image.into_rgba16().into_raw();
        let warning =
            crate::color::normalize16(&mut rgba16, icc.as_deref(), cmyk).or(metadata_warning);
        let levels = crate::levels::generate16(size, &rgba16)?;
        Ok(Pixels {
            size,
            rgba: vec![],
            rgba16: Some(rgba16),
            levels,
            warning,
            animation_delay: None,
            ..Default::default()
        })
    } else {
        let mut rgba = image.into_rgba8().into_raw();
        let warning =
            crate::color::normalize8(&mut rgba, icc.as_deref(), cmyk).or(metadata_warning);
        let levels = crate::levels::generate(size, &rgba)?;
        Ok(Pixels {
            size,
            rgba,
            levels,
            warning,
            rgba16: None,
            animation_delay: None,
            ..Default::default()
        })
    }
}

fn checked_output_bytes(width: u32, height: u32, bytes_per_pixel: u64) -> Result<(), LoadError> {
    if u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(bytes_per_pixel))
        .is_none_or(|n| n > MAX_DECODED_BYTES)
    {
        return Err(LoadError::ResourceLimit(
            "Normalized source exceeds 512 MiB (RGBA16 uses 8 bytes/pixel)".into(),
        ));
    }
    Ok(())
}

fn checked_size(width: u32, height: u32, dimension_limit: u32) -> Result<[usize; 2], LoadError> {
    let bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4));
    if width == 0
        || height == 0
        || width > dimension_limit
        || height > dimension_limit
        || bytes.is_none_or(|bytes| bytes > MAX_DECODED_BYTES)
    {
        return Err(LoadError::ResourceLimit(format!(
            "{width} × {height}; maximum side {dimension_limit}, decoded RGBA8 budget {} MiB",
            MAX_DECODED_BYTES / (1024 * 1024)
        )));
    }
    let width = usize::try_from(width)
        .map_err(|_| LoadError::ResourceLimit("width is not addressable".into()))?;
    let height = usize::try_from(height)
        .map_err(|_| LoadError::ResourceLimit("height is not addressable".into()))?;
    Ok([width, height])
}

mod avif;
mod document;
mod jxl;
pub mod svg;

#[cfg(test)]
mod format_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folder::tests::TempDir;

    fn tagged_png(path: &Path, icc: Vec<u8>) {
        use image::{ExtendedColorType, ImageEncoder};
        let mut encoder = image::codecs::png::PngEncoder::new(File::create(path).unwrap());
        encoder.set_icc_profile(icc).unwrap();
        // 256x256 opaque blocks keep the ICC shift visible at human-friendly size:
        // unit-test color (153,102,51) at full alpha, plus a lighter sweep column so
        // the P3-vs-untagged difference is apparent across tones, not one dot.
        let size = 256u32;
        let mut row = Vec::with_capacity((size * 4) as usize);
        for x in 0..size {
            let lift = (x * 64 / size) as u8;
            row.extend_from_slice(&[
                153u8.saturating_add(lift),
                102u8.saturating_add(lift),
                51u8.saturating_add(lift),
                255,
            ]);
        }
        let mut pixels = Vec::with_capacity((size * size * 4) as usize);
        for _ in 0..size {
            pixels.extend_from_slice(&row);
        }
        encoder
            .write_image(&pixels, size, size, ExtendedColorType::Rgba8)
            .unwrap();
    }

    #[test]
    fn embedded_icc_is_read_before_decoder_consumption() {
        let dir = TempDir::new();
        for (name, icc, expected, warning) in [
            (
                "srgb",
                moxcms::ColorProfile::new_srgb().encode().unwrap(),
                [153, 102, 51, 255],
                false,
            ),
            (
                "p3",
                moxcms::ColorProfile::new_display_p3().encode().unwrap(),
                [162, 99, 39, 255],
                false,
            ),
            (
                "lut",
                crate::color::tests::srgb_lut(),
                [153, 102, 51, 255],
                false,
            ),
            (
                "invalid",
                b"invalid ICC".to_vec(),
                [153, 102, 51, 255],
                true,
            ),
        ] {
            let path = dir.0.join(format!("{name}.png"));
            tagged_png(&path, icc);
            let pixels = load(&path, MAX_DIMENSION).unwrap();
            assert_eq!(pixels.warning.is_some(), warning);
            assert_eq!(pixels.size, [256, 256]);
            // Column 0 (lift 0) carries the exact tag-converted base color.
            for (actual, expected) in pixels.rgba[..4].iter().zip(expected) {
                assert!(actual.abs_diff(expected) <= 2, "{name}: {:?}", actual);
            }
            // Alpha is fully opaque everywhere; the column lift survives decode as a
            // monotone rise (CMS roundtrip through linear light may quantize a few
            // codes, so exact per-column equality is not asserted).
            assert!(pixels.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255));
            let reds: Vec<u8> = pixels.rgba[..256 * 4]
                .chunks_exact(4)
                .map(|pixel| pixel[0])
                .collect();
            assert!(reds.windows(2).all(|pair| pair[1] >= pair[0]));
            assert!(reds[255].saturating_sub(reds[0]) >= 40, "{name}: {reds:?}");
        }
    }

    #[test]
    fn png_and_tiff_gradient_preserve_more_than_8bit_through_tiles_and_copy() {
        use std::collections::HashSet;
        let dir = TempDir::new();
        let image = image::ImageBuffer::<image::Rgba<u16>, _>::from_fn(4096, 1, |x, _| {
            image::Rgba([x as u16 * 16; 4])
        });
        for extension in ["png", "tiff"] {
            let path = dir.0.join(format!("gradient.{extension}"));
            image.save(&path).unwrap();
            let pixels = load(&path, MAX_DIMENSION).unwrap();
            assert_eq!(pixels.rgba16.as_ref().unwrap(), image.as_raw());
            assert!(pixels.rgba.is_empty());
            assert!(pixels.levels.iter().all(|l| l.half));
            for level in &pixels.levels[..3] {
                let unique: HashSet<_> = level.rgba.chunks_exact(8).map(|p| [p[0], p[1]]).collect();
                assert!(unique.len() > 256, "unique={}", unique.len());
            }
            let tile = crate::tiles::tile(
                pixels.size,
                crate::tiles::Key {
                    level: 0,
                    x: 0,
                    y: 0,
                },
                1022,
            );
            let data = crate::tiles::gutter(&pixels.levels[0], tile).unwrap();
            assert_eq!(data.len(), tile.bytes_at_depth(8));
            assert_eq!(&data[8..16], &pixels.levels[0].rgba[..8]);
            let mut transform = crate::transform::Transform::default();
            transform.apply(crate::transform::Operation::Mirror);
            let copy = transform.materialize(&pixels).unwrap();
            assert_eq!(copy.rgba[..4], [255; 4]);
            assert_eq!(&copy.rgba[copy.rgba.len() - 4..], &[0; 4]);
            assert_eq!(
                pixels.accounted_bytes(),
                pixels.source_bytes() + pixels.levels.iter().map(|l| l.rgba.len()).sum::<usize>()
            );
        }
        assert!(checked_output_bytes(16384, 4096, 8).is_ok());
        assert!(checked_output_bytes(16384, 4097, 8).is_err());
    }

    // Original constant CMYK baseline JPEG: 1x1, four 8x8 blocks, all DC/AC
    // zero (sample 128), custom one-symbol DC/EOB Huffman tables. No legacy asset.
    fn cmyk_jpeg() -> Vec<u8> {
        let mut data = vec![0xff, 0xd8];
        let mut segment = |marker: u8, payload: &[u8]| {
            data.extend([0xff, marker]);
            data.extend(((payload.len() + 2) as u16).to_be_bytes());
            data.extend(payload);
        };
        let mut quant = vec![0];
        quant.extend([1; 64]);
        segment(0xdb, &quant);
        segment(
            0xc0,
            &[
                8, 0, 1, 0, 1, 4, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0, 4, 0x11, 0,
            ],
        );
        for class in [0, 0x10] {
            let mut huffman = vec![class, 1];
            huffman.extend([0; 15]);
            huffman.push(0);
            segment(0xc4, &huffman);
        }
        segment(0xda, &[4, 1, 0, 2, 0, 3, 0, 4, 0, 0, 63, 0]);
        data.extend([0, 0xff, 0xd9]);
        data
    }

    #[test]
    fn real_cmyk_jpeg_displays_with_explicit_approximate_label() {
        let dir = TempDir::new();
        let path = dir.0.join("cmyk.jpg");
        std::fs::write(&path, cmyk_jpeg()).unwrap();
        let pixels = load(&path, MAX_DIMENSION).unwrap();
        assert_eq!(pixels.size, [1, 1]);
        assert_eq!(pixels.rgba.len(), 4);
        assert_eq!(pixels.warning.as_deref(), Some(crate::color::CMYK_PREVIEW));
    }

    #[test]
    fn float_tiff_uses_fixed_tone_mapping_before_half_presentation() {
        let dir = TempDir::new();
        let path = dir.0.join("float.tiff");
        let image = image::ImageBuffer::<image::Rgba<f32>, _>::from_fn(5, 1, |x, _| {
            image::Rgba([x as f32, x as f32, x as f32, 1.0])
        });
        image.save(&path).unwrap();
        let a = load(&path, MAX_DIMENSION).unwrap();
        let b = load(&path, MAX_DIMENSION).unwrap();
        assert_eq!(a.rgba16, b.rgba16);
        assert_eq!(
            a.rgba16.as_ref().unwrap(),
            &crate::color::float_to_sdr(image.as_raw()).unwrap()
        );
        assert!(a.warning.as_ref().unwrap().contains("scene-linear sRGB"));
        assert!(a.levels[0].half);
    }

    #[test]
    #[ignore = "Creates retained original fixtures for manual release smokes"]
    fn write_color_smoke_fixtures() {
        let dir = std::env::temp_dir().join(format!("glance-color-smoke-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        tagged_png(
            &dir.join("p3.png"),
            moxcms::ColorProfile::new_display_p3().encode().unwrap(),
        );
        image::RgbaImage::from_pixel(256, 256, image::Rgba([153, 102, 51, 255]))
            .save(dir.join("untagged.png"))
            .unwrap();
        tagged_png(&dir.join("invalid.png"), b"invalid ICC".to_vec());
        image::ImageBuffer::<image::Rgba<u16>, _>::from_fn(4096, 64, |x, _| {
            image::Rgba([x as u16 * 16, x as u16 * 16, x as u16 * 16, 65535])
        })
        .save(dir.join("gradient.png"))
        .unwrap();
        std::fs::write(dir.join("cmyk.jpg"), cmyk_jpeg()).unwrap();
        image::ImageBuffer::<image::Rgba<f32>, _>::from_fn(256, 32, |x, _| {
            image::Rgba([x as f32 / 32.0, x as f32 / 32.0, x as f32 / 32.0, 1.0])
        })
        .save(dir.join("float.tiff"))
        .unwrap();
        println!("Retained original color fixtures: {}", dir.display());
    }

    #[test]
    fn real_png_and_jpeg_decode() {
        let dir = TempDir::new();
        let image = image::RgbImage::from_pixel(3, 2, image::Rgb([22, 44, 66]));
        for extension in ["png", "jpg"] {
            let path = dir.0.join(format!("fixture.{extension}"));
            image.save(&path).unwrap();
            let decoded = load(&path, MAX_DIMENSION).unwrap();
            assert_eq!(decoded.size, [3, 2]);
            assert_eq!(decoded.rgba.len(), 24);
            assert!(decoded.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255));
        }
        let decoded = load(&dir.0.join("fixture.png"), MAX_DIMENSION).unwrap();
        assert_eq!(&decoded.rgba[..4], &[22, 44, 66, 255]);
    }

    #[test]
    fn large_photo_fits_decoded_budget() {
        assert_eq!(
            checked_size(7688, 4976, MAX_DIMENSION).unwrap(),
            [7688, 4976]
        );
    }

    #[test]
    fn decoded_budget_boundary_is_checked_without_allocating() {
        let width = MAX_DIMENSION;
        let height = (MAX_DECODED_BYTES / (u64::from(width) * 4)) as u32;
        assert!(checked_size(width, height, MAX_DIMENSION).is_ok());
        let error = checked_size(width, height + 1, MAX_DIMENSION).unwrap_err();
        assert!(matches!(error, LoadError::ResourceLimit(_)));
        let message = error.to_string();
        assert!(message.contains(&format!(
            "decoded RGBA8 budget {} MiB",
            MAX_DECODED_BYTES / (1024 * 1024)
        )));
        assert!(message.contains(&format!("maximum side {MAX_DIMENSION}")));
    }

    #[test]
    fn errors_are_recoverable_and_limits_are_checked() {
        let dir = TempDir::new();
        let corrupt = dir.0.join("bad.png");
        std::fs::write(&corrupt, b"broken").unwrap();
        assert!(matches!(
            load(&corrupt, MAX_DIMENSION),
            Err(LoadError::Malformed(_))
        ));
        assert!(matches!(
            load(&dir.0.join("missing.jpg"), MAX_DIMENSION),
            Err(LoadError::Io(_))
        ));
        assert!(matches!(
            load(&dir.0.join("file.xcf"), MAX_DIMENSION),
            Err(LoadError::Unsupported)
        ));
        assert!(checked_size(u32::MAX, u32::MAX, u32::MAX).is_err());
        assert!(checked_size(0, 10, MAX_DIMENSION).is_err());
        assert!(checked_size(MAX_DIMENSION + 1, 1, MAX_DIMENSION).is_err());
        assert!(checked_size(3000, 2000, MAX_DIMENSION).is_ok());
        image::RgbImage::new(3, 2)
            .save(dir.0.join("wide.png"))
            .unwrap();
        assert_eq!(load(&dir.0.join("wide.png"), 2).unwrap().size, [3, 2]);
    }
}
