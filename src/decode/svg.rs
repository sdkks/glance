use super::{LoadError, MAX_DIMENSION, MAX_ENCODED_BYTES, Pixels, checked_size};
use resvg::{tiny_skia, usvg};
use std::{
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) fn decode(encoded: &[u8], gzip: bool) -> Result<Pixels, LoadError> {
    let xml = if gzip {
        let mut xml = Vec::new();
        flate2::read::GzDecoder::new(encoded)
            .take(MAX_ENCODED_BYTES + 1)
            .read_to_end(&mut xml)
            .map_err(|e| LoadError::Malformed(format!("SVGZ: {e}")))?;
        if xml.len() as u64 > MAX_ENCODED_BYTES {
            return Err(LoadError::ResourceLimit(
                "SVGZ: expanded XML exceeds 64 MiB".into(),
            ));
        }
        xml
    } else {
        encoded.to_vec()
    };
    let text = std::str::from_utf8(&xml).map_err(|e| LoadError::Malformed(format!("SVG: {e}")))?;
    let doc =
        roxmltree::Document::parse(text).map_err(|e| LoadError::Malformed(format!("SVG: {e}")))?;
    for node in doc.descendants().filter(|n| n.is_element()) {
        if matches!(
            node.tag_name().name(),
            "script" | "animate" | "animateMotion" | "animateTransform" | "set" | "foreignObject"
        ) || node.attributes().any(|a| a.name().starts_with("on"))
        {
            return Err(LoadError::UnsupportedVariant(
                "SVG: scripts, event handlers, animation and foreignObject are not supported"
                    .into(),
            ));
        }
    }
    let rejected = Arc::new(AtomicBool::new(false));
    let data_rejected = rejected.clone();
    let string_rejected = rejected.clone();
    let default_data = usvg::ImageHrefResolver::default_data_resolver();
    let resolver = usvg::ImageHrefResolver {
        resolve_data: Box::new(move |mime, data, opts| {
            // Validate the embedded raster before resvg's decoder allocates it.
            // Nested SVG and external files are intentionally outside this profile.
            let valid = (|| {
                let mut reader = image::ImageReader::new(std::io::Cursor::new(data.as_slice()))
                    .with_guessed_format()
                    .ok()?;
                if !matches!(
                    reader.format(),
                    Some(
                        image::ImageFormat::Png
                            | image::ImageFormat::Jpeg
                            | image::ImageFormat::Gif
                            | image::ImageFormat::WebP
                    )
                ) {
                    return None;
                }
                let mut limits = image::Limits::default();
                limits.max_image_width = Some(4096);
                limits.max_image_height = Some(4096);
                limits.max_alloc = Some(64 * 1024 * 1024);
                reader.limits(limits);
                let decoder = reader.into_decoder().ok()?;
                use image::ImageDecoder;
                let (w, h) = decoder.dimensions();
                if w == 0
                    || h == 0
                    || w > 4096
                    || h > 4096
                    || u64::from(w) * u64::from(h) * 4 > 64 * 1024 * 1024
                {
                    return None;
                }
                drop(decoder);
                default_data(mime, data, opts)
            })();
            if valid.is_none() {
                data_rejected.store(true, Ordering::Relaxed);
            }
            valid
        }),
        resolve_string: Box::new(move |_, _| {
            string_rejected.store(true, Ordering::Relaxed);
            None
        }),
    };
    let mut options = usvg::Options {
        image_href_resolver: resolver,
        ..Default::default()
    };
    options.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_str(text, &options)
        .map_err(|e| LoadError::Malformed(format!("SVG: {e}")))?;
    if rejected.load(Ordering::Relaxed) {
        return Err(LoadError::UnsupportedVariant(
            "SVG: external/nested images or embedded raster exceeding 4096 pixels / 64 MiB".into(),
        ));
    }
    rasterize(&tree, 1.0)
}

fn rasterize(tree: &usvg::Tree, scale: f32) -> Result<Pixels, LoadError> {
    let width = (tree.size().width() * scale).ceil() as u32;
    let height = (tree.size().height() * scale).ceil() as u32;
    let size = checked_size(width, height, MAX_DIMENSION)?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| LoadError::ResourceLimit("SVG: raster allocation failed".into()))?;
    resvg::render(
        tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia is premultiplied encoded RGB, unlike our straight source seam.
    let mut rgba = pixmap.take();
    for p in rgba.chunks_exact_mut(4) {
        let alpha = u32::from(p[3]);
        for c in &mut p[..3] {
            *c = (u32::from(*c) * 255 + alpha / 2)
                .checked_div(alpha)
                .unwrap_or(0)
                .min(255) as u8;
        }
    }
    let levels = crate::levels::generate(size, &rgba)?;
    Ok(Pixels {
        size,
        rgba,
        levels,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const XML: &[u8] = b"<svg xmlns='http://www.w3.org/2000/svg' width='20' height='10'><rect width='10' height='10' fill='#ff0000' fill-opacity='.5'/></svg>";
    #[test]
    fn analytic_shapes_at_two_scales_and_copy() {
        let tree = usvg::Tree::from_data(XML, &usvg::Options::default()).unwrap();
        for scale in [1.0, 2.0] {
            let p = rasterize(&tree, scale).unwrap();
            assert_eq!(p.size, [(20.0 * scale) as usize, (10.0 * scale) as usize]);
            assert_eq!(&p.rgba[..4], &[255, 0, 0, 128]);
            assert_eq!(&p.rgba[(p.size[0] - 1) * 4..p.size[0] * 4], &[0, 0, 0, 0]);
        }
        let p = decode(XML, false).unwrap();
        let copy = crate::transform::Transform::default()
            .materialize(&p)
            .unwrap();
        assert_eq!(copy.rgba, p.rgba);
        assert_eq!(copy.size, [20, 10]);
    }
    #[test]
    fn gzip_and_named_rejections() {
        use std::io::Write;
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(XML).unwrap();
        let bytes = gzip.finish().unwrap();
        assert_eq!(
            decode(&bytes, true).unwrap().rgba,
            decode(XML, false).unwrap().rgba
        );
        assert!(
            decode(&bytes[..10], true)
                .err()
                .unwrap()
                .to_string()
                .contains("SVGZ")
        );
        assert!(
            decode(b"<svg", false)
                .err()
                .unwrap()
                .to_string()
                .contains("SVG")
        );
        assert!(matches!(
            decode(b"<svg><script/></svg>", false),
            Err(LoadError::UnsupportedVariant(_))
        ));
        assert!(matches!(decode(b"<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'><image href='/etc/passwd' width='10' height='10'/></svg>", false), Err(LoadError::UnsupportedVariant(_))));
        for ext in ["svg", "SVGZ"] {
            assert!(crate::folder::eligible(std::path::Path::new(&format!(
                "a.{ext}"
            ))));
        }
    }
}
