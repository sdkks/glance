use moxcms::{ColorProfile, DataColorSpace, Layout, RenderingIntent, TransformOptions};

// Codec output is straight encoded RGB. Alpha bypasses the CMS entirely.
// A failed transform is transactional: keep the original pixels and disclose sRGB fallback.
pub const FALLBACK: &str = "ICC fallback: unsupported/invalid profile; assuming sRGB";
pub const CMYK_PREVIEW: &str = "CMYK approximate SDR preview (not profile-accurate)";
pub const MAX_PROFILE_BYTES: usize = 4 * 1024 * 1024;

// Fixed exposure, scene-linear sRGB, white=4. Invalid radiance contributes zero;
// +Inf saturates to a finite white-point radiance so all arithmetic stays finite.
pub fn tone_map(rgb: [f32; 3]) -> [f32; 3] {
    let rgb = rgb.map(|v| {
        if v == f32::INFINITY {
            4.0
        } else if v.is_finite() {
            v.clamp(0.0, 1.0e20)
        } else {
            0.0
        }
    });
    let y = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
    if y == 0.0 {
        return [0.0; 3];
    }
    let scale = (1.0 + y / 16.0) / (1.0 + y);
    rgb.map(|v| (v * scale).clamp(0.0, 1.0))
}

pub fn float_to_sdr(rgba: &[f32]) -> Result<Vec<u16>, crate::decode::LoadError> {
    let mut output = Vec::new();
    output.try_reserve_exact(rgba.len()).map_err(|_| {
        crate::decode::LoadError::ResourceLimit("Float SDR allocation failed".into())
    })?;
    for p in rgba.chunks_exact(4) {
        let rgb = tone_map([p[0], p[1], p[2]]);
        output.extend(rgb.map(|v| (crate::levels::encoded_unit(v) * 65535.0).round() as u16));
        let alpha = if p[3].is_finite() {
            p[3].clamp(0.0, 1.0)
        } else {
            0.0
        };
        output.push((alpha * 65535.0).round() as u16);
    }
    Ok(output)
}

fn profile(icc: &[u8]) -> Result<ColorProfile, ()> {
    if icc.len() > MAX_PROFILE_BYTES {
        return Err(());
    }
    let profile = ColorProfile::new_from_slice_with_options(
        icc,
        moxcms::ParsingOptions {
            max_profile_size: MAX_PROFILE_BYTES + 1,
            max_allowed_clut_size: MAX_PROFILE_BYTES,
            max_allowed_trc_size: 65536,
        },
    )
    .map_err(|_| ())?;
    if profile.color_space != DataColorSpace::Rgb {
        return Err(());
    }
    Ok(profile)
}

fn options() -> TransformOptions {
    TransformOptions {
        rendering_intent: RenderingIntent::RelativeColorimetric,
        prefer_fixed_point: false,
        ..Default::default()
    }
}

pub fn normalize8(rgba: &mut Vec<u8>, icc: Option<&[u8]>, cmyk: bool) -> Option<String> {
    if cmyk {
        return Some(CMYK_PREVIEW.into());
    }
    let icc = icc?;
    let converted = (|| {
        let source = profile(icc)?;
        let transform = source
            .create_transform_8bit(
                Layout::Rgba,
                &ColorProfile::new_srgb(),
                Layout::Rgba,
                options(),
            )
            .map_err(|_| ())?;
        let mut output = Vec::new();
        output.try_reserve_exact(rgba.len()).map_err(|_| ())?;
        output.resize(rgba.len(), 0);
        transform.transform(rgba, &mut output).map_err(|_| ())?;
        for (src, dst) in rgba.chunks_exact(4).zip(output.chunks_exact_mut(4)) {
            dst[3] = src[3];
        }
        Ok::<_, ()>(output)
    })();
    match converted {
        Ok(output) => {
            *rgba = output;
            None
        }
        Err(()) => Some(FALLBACK.into()),
    }
}

pub fn normalize16(rgba: &mut Vec<u16>, icc: Option<&[u8]>, cmyk: bool) -> Option<String> {
    if cmyk {
        return Some(CMYK_PREVIEW.into());
    }
    let icc = icc?;
    let converted = (|| {
        let source = profile(icc)?;
        let transform = source
            .create_transform_16bit(
                Layout::Rgba,
                &ColorProfile::new_srgb(),
                Layout::Rgba,
                options(),
            )
            .map_err(|_| ())?;
        let mut output = Vec::new();
        output.try_reserve_exact(rgba.len()).map_err(|_| ())?;
        output.resize(rgba.len(), 0);
        transform.transform(rgba, &mut output).map_err(|_| ())?;
        for (src, dst) in rgba.chunks_exact(4).zip(output.chunks_exact_mut(4)) {
            dst[3] = src[3];
        }
        Ok::<_, ()>(output)
    })();
    match converted {
        Ok(output) => {
            *rgba = output;
            None
        }
        Err(()) => Some(FALLBACK.into()),
    }
}

// image's JPEG wrapper hides the original CMYK/YCCK colorspace. Four SOF
// components identify both before its deliberate approximate conversion to RGB.
// Walk length-delimited header segments, never search arbitrary compressed data.
pub fn jpeg_cmyk(bytes: &[u8]) -> bool {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return false;
    }
    let mut at = 2;
    while at + 1 < bytes.len() {
        if bytes[at] != 0xff {
            return false;
        }
        while at < bytes.len() && bytes[at] == 0xff {
            at += 1;
        }
        let Some(&marker) = bytes.get(at) else {
            return false;
        };
        at += 1;
        if marker == 0xda || marker == 0xd9 {
            return false;
        }
        if marker == 0x01 || (0xd0..=0xd8).contains(&marker) {
            continue;
        }
        let Some(length) = bytes.get(at..at + 2) else {
            return false;
        };
        let length = usize::from(u16::from_be_bytes([length[0], length[1]]));
        if length < 2 || at + length > bytes.len() {
            return false;
        }
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            return length >= 8 && bytes[at + 7] == 4;
        }
        at += length;
    }
    false
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // Original procedural ICC fixture: RGB->D50 XYZ mft2 CLUT, sampled from
    // the public sRGB colorant matrix and IEC sRGB transfer, no borrowed asset.
    pub fn srgb_lut() -> Vec<u8> {
        use moxcms::{LutDataType, LutStore, LutType, LutWarehouse, Matrix3d};
        let mut profile = ColorProfile::new_srgb();
        let matrix = profile.colorant_matrix().v;
        let mut clut = Vec::new();
        for r in 0..33 {
            for g in 0..33 {
                for b in 0..33 {
                    let rgb = [r, g, b].map(|v| crate::levels::linear_unit(v as f32 / 32.0) as f64);
                    for row in matrix {
                        let xyz: f64 = row.iter().zip(rgb).map(|(a, b)| a * b).sum();
                        clut.push((xyz * 32768.0).round().clamp(0.0, 65535.0) as u16);
                    }
                }
            }
        }
        let tables: Vec<u16> = (0..3).flat_map(|_| (0..=255).map(|v| v * 257)).collect();
        profile.lut_a_to_b_colorimetric = Some(LutWarehouse::Lut(LutDataType {
            num_input_channels: 3,
            num_output_channels: 3,
            num_clut_grid_points: 33,
            matrix: Matrix3d {
                v: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            },
            num_input_table_entries: 256,
            num_output_table_entries: 256,
            input_table: LutStore::Store16(tables.clone()),
            output_table: LutStore::Store16(tables),
            clut_table: LutStore::Store16(clut),
            lut_type: LutType::Lut16,
        }));
        profile.encode().unwrap()
    }

    #[test]
    fn srgb_lut_profile_matches_matrix_with_interpolation_tolerance() {
        let icc = srgb_lut();
        let mut pixels = vec![
            128, 128, 128, 255, 153, 102, 51, 0, 255, 255, 255, 100, 0, 0, 0, 255,
        ];
        let before = pixels.clone();
        assert_eq!(normalize8(&mut pixels, Some(&icc), false), None);
        for (a, b) in pixels.iter().zip(before) {
            assert!(a.abs_diff(b) <= 2, "{pixels:?}");
        }
    }

    #[test]
    fn fixed_tone_map_matches_analytic_ramps_and_is_frame_stable() {
        for v in [0.0f32, 0.25, 1.0, 2.0, 4.0, 16.0] {
            let expected = (v * (1.0 + v / 16.0) / (1.0 + v)).min(1.0);
            let result = tone_map([v; 3]);
            assert!(result.iter().all(|c| (c - expected).abs() < 1e-6));
            assert_eq!(result, tone_map([v; 3]));
        }
        let a = float_to_sdr(&[1.0, 2.0, 4.0, 0.25]).unwrap();
        assert_eq!(a, float_to_sdr(&[1.0, 2.0, 4.0, 0.25]).unwrap());
        assert_eq!(a[3], 16384);
        assert_eq!(tone_map([f32::NAN, f32::NEG_INFINITY, -1.0]), [0.0; 3]);
        assert!(
            tone_map([f32::INFINITY, 1e30, 0.0])
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
        );
        assert_eq!(float_to_sdr(&[0.0, 0.0, 0.0, f32::NAN]).unwrap()[3], 0);
    }

    #[test]
    fn untagged_and_srgb_are_identical_and_alpha_bypasses_cms() {
        let original: Vec<u8> = (0..=255).flat_map(|v| [v, v, v, 255 - v]).collect();
        let mut pixels = original.clone();
        assert_eq!(normalize8(&mut pixels, None, false), None);
        assert_eq!(pixels, original);
        let icc = ColorProfile::new_srgb().encode().unwrap();
        assert_eq!(normalize8(&mut pixels, Some(&icc), false), None);
        assert_eq!(pixels, original);
        let mut high: Vec<u16> = original.iter().map(|v| u16::from(*v) * 257).collect();
        let before = high.clone();
        assert_eq!(normalize16(&mut high, Some(&icc), false), None);
        let maximum = high
            .iter()
            .zip(before)
            .map(|(a, b)| a.abs_diff(b))
            .max()
            .unwrap();
        assert!(maximum <= 16, "16-bit sRGB CMS quantization: {maximum}");
    }

    #[test]
    fn display_p3_matches_independent_d65_matrix_reference() {
        // Reference: published Display-P3->XYZ->sRGB D65 matrices, sRGB transfer.
        // Interior gamut avoids clipping: [0.6,0.4,0.2] -> [0.6358,0.3887,0.1537].
        let icc = ColorProfile::new_display_p3().encode().unwrap();
        let mut pixels = vec![153, 102, 51, 73];
        assert_eq!(normalize8(&mut pixels, Some(&icc), false), None);
        for (a, b) in pixels[..3].iter().zip([162u8, 99, 39]) {
            assert!(a.abs_diff(b) <= 1, "{pixels:?}");
        }
        assert_eq!(pixels[3], 73);
    }

    #[test]
    fn invalid_and_non_rgb_profiles_fall_back_without_mutation() {
        let original = vec![100, 50, 10, 0];
        let mut pixels = original.clone();
        assert_eq!(
            normalize8(&mut pixels, Some(b"bad"), false).as_deref(),
            Some(FALLBACK)
        );
        assert_eq!(pixels, original);
        let mut icc = ColorProfile::new_srgb().encode().unwrap();
        icc[16..20].copy_from_slice(b"CMYK");
        assert_eq!(
            normalize8(&mut pixels, Some(&icc), false).as_deref(),
            Some(FALLBACK)
        );
        assert_eq!(pixels, original);
        assert_eq!(
            normalize8(&mut pixels, Some(b"bad"), true).as_deref(),
            Some(CMYK_PREVIEW)
        );
        assert_eq!(pixels, original);
    }

    #[test]
    fn cmyk_detection_uses_sof_not_payload_and_handles_truncation() {
        let header = [
            0xff, 0xd8, 0xff, 0xc0, 0, 20, 8, 0, 1, 0, 1, 4, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0, 4,
            0x11, 0,
        ];
        assert!(jpeg_cmyk(&header));
        for n in 0..header.len() {
            assert!(!jpeg_cmyk(&header[..n]));
        }
        let mut rgb = header;
        rgb[11] = 3;
        assert!(!jpeg_cmyk(&rgb));
    }
}
