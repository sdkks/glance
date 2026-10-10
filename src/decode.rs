use std::{
    fmt,
    fs::File,
    io::{Cursor, Read},
    path::Path,
};

use image::{DynamicImage, ImageDecoder, ImageError, ImageFormat, ImageReader, Limits};

pub const MAX_ENCODED_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_DECODED_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_DIMENSION: u32 = 16384;

pub struct Pixels {
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Malformed(String),
    Unsupported,
    ResourceLimit(String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "Could not read file: {error}"),
            Self::Malformed(error) => write!(f, "Could not decode image: {error}"),
            Self::Unsupported => write!(
                f,
                "Unsupported format. This version opens JPEG and PNG only."
            ),
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
            error => Self::Malformed(error.to_string()),
        }
    }
}

pub fn load(path: &Path, texture_limit: u32) -> Result<Pixels, LoadError> {
    let extension = path
        .extension()
        .map(|ext| ext.as_encoded_bytes())
        .unwrap_or_default();
    let format = if extension.eq_ignore_ascii_case(b"png") {
        ImageFormat::Png
    } else if extension.eq_ignore_ascii_case(b"jpg") || extension.eq_ignore_ascii_case(b"jpeg") {
        ImageFormat::Jpeg
    } else {
        return Err(LoadError::Unsupported);
    };
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_ENCODED_BYTES {
        return Err(LoadError::ResourceLimit(
            "encoded input exceeds 64 MiB".into(),
        ));
    }
    // Bound reads even if the file grows after the metadata check.
    let mut encoded = Vec::new();
    file.take(MAX_ENCODED_BYTES + 1).read_to_end(&mut encoded)?;
    if encoded.len() as u64 > MAX_ENCODED_BYTES {
        return Err(LoadError::ResourceLimit(
            "encoded input exceeds 64 MiB".into(),
        ));
    }
    let dimension_limit = texture_limit.min(MAX_DIMENSION);
    let mut limits = Limits::default();
    limits.max_image_width = Some(dimension_limit);
    limits.max_image_height = Some(dimension_limit);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    let mut reader = ImageReader::with_format(Cursor::new(encoded), format);
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    let size = checked_size(width, height, dimension_limit)?;
    if decoder.total_bytes() > MAX_DECODED_BYTES {
        return Err(LoadError::ResourceLimit(format!(
            "decoded samples exceed {} MiB; decoded RGBA8 budget {} MiB, maximum side {dimension_limit}",
            MAX_DECODED_BYTES / (1024 * 1024),
            MAX_DECODED_BYTES / (1024 * 1024)
        )));
    }
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    // RGBA8 is explicitly temporary presentation; high-depth/color handling lands later.
    let rgba = image.into_rgba8();
    let size = if rgba.width() == width && rgba.height() == height {
        size
    } else {
        checked_size(rgba.width(), rgba.height(), dimension_limit)?
    };
    Ok(Pixels {
        size,
        rgba: rgba.into_raw(),
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folder::tests::TempDir;

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
        assert!(matches!(
            load(&dir.0.join("wide.png"), 2),
            Err(LoadError::ResourceLimit(_))
        ));
    }
}
