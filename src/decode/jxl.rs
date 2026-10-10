use super::{
    LoadError, MAX_DECODED_BYTES, MAX_DIMENSION, Pixels, checked_output_bytes, checked_size,
};

pub(super) fn decode(encoded: &[u8]) -> Result<Pixels, LoadError> {
    let malformed = |e| LoadError::Malformed(format!("JXL: {e}"));
    let mut image = jxl_oxide::JxlImage::builder()
        .alloc_tracker(jxl_oxide::AllocTracker::with_limit(
            MAX_DECODED_BYTES as usize,
        ))
        .read(encoded)
        .map_err(malformed)?;
    let size = checked_size(image.width(), image.height(), MAX_DIMENSION)?;
    checked_output_bytes(image.width(), image.height(), 8)?;
    if image.image_header().metadata.animation.is_some() {
        return Err(LoadError::UnsupportedVariant(
            "JXL: sequences are not supported yet".into(),
        ));
    }
    if image.hdr_type().is_some() {
        return Err(LoadError::UnsupportedVariant(
            "JXL: HDR transfer is not supported yet".into(),
        ));
    }
    if !image.is_loading_done() || image.num_loaded_keyframes() != 1 {
        return Err(LoadError::Malformed("JXL: incomplete still image".into()));
    }
    // Core rendering performs color management and the encoded orientation.
    // Never apply EXIF orientation or the original ICC again to this output.
    image.request_color_encoding(jxl_oxide::EnumColourEncoding::srgb(
        jxl_oxide::RenderingIntent::Relative,
    ));
    let render = image.render_frame(0).map_err(malformed)?;
    let mut stream = render.stream();
    let channels = stream.channels() as usize;
    if !(1..=4).contains(&channels) || image.pixel_format().has_black() {
        return Err(LoadError::UnsupportedVariant(
            "JXL: CMYK/extra channel layout".into(),
        ));
    }
    let mut samples = vec![0u16; size[0] * size[1] * channels];
    if stream.write_to_buffer(&mut samples) != samples.len() {
        return Err(LoadError::Malformed("JXL: incomplete pixel stream".into()));
    }
    let mut rgba16 = Vec::with_capacity(size[0] * size[1] * 4);
    for p in samples.chunks_exact(channels) {
        match channels {
            1 => rgba16.extend([p[0], p[0], p[0], 65535]),
            2 => rgba16.extend([p[0], p[0], p[0], p[1]]),
            3 => rgba16.extend([p[0], p[1], p[2], 65535]),
            4 => rgba16.extend_from_slice(p),
            _ => unreachable!(),
        }
    }
    let levels = crate::levels::generate16(size, &rgba16)?;
    Ok(Pixels {
        size,
        rgba16: Some(rgba16),
        levels,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_still_depth_color_and_clipboard() {
        let p = decode(include_bytes!("../../assets/test/web/still.jxl")).unwrap();
        assert_eq!(p.size, [3, 2]);
        let source = p.rgba16.as_ref().unwrap();
        assert!(source[0].abs_diff(22 * 257) <= 2);
        assert!(source[1].abs_diff(44 * 257) <= 2);
        assert!(source[2].abs_diff(66 * 257) <= 2);
        assert!(source.chunks_exact(4).all(|p| p[3] == 65535));
        assert!(p.levels[0].half);
        let copy = crate::transform::Transform::default()
            .materialize(&p)
            .unwrap();
        assert_eq!(copy.rgba[..4], [22, 44, 66, 255]);
        assert!(crate::folder::eligible(std::path::Path::new("still.JXL")));
    }

    #[test]
    fn truncated_is_named() {
        assert!(decode(&[]).err().unwrap().to_string().contains("JXL"));
        let bytes = include_bytes!("../../assets/test/web/still.jxl");
        assert!(decode(&bytes[..bytes.len() / 2]).is_err());
    }
}
