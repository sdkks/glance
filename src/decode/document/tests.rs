use super::*;
use image::ImageEncoder;
use std::io::Write;

// Original analytic pixels and containers, authored here; no borrowed assets.
fn png() -> Vec<u8> {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            &[22, 44, 66, 255, 88, 99, 111, 128],
            2,
            1,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    bytes
}
fn zip_entries(entries: &[(&str, &[u8])], method: zip::CompressionMethod) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in entries {
        writer
            .start_file(
                *name,
                zip::write::SimpleFileOptions::default().compression_method(method),
            )
            .unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
fn fixture(mode: u16, compression: u16, channels: u16) -> Vec<u8> {
    // Minimal PSD v1, 2x1, planar samples; psd has no writer API.
    let mut bytes = b"8BPS".to_vec();
    bytes.extend(1u16.to_be_bytes());
    bytes.extend([0; 6]);
    bytes.extend(channels.to_be_bytes());
    bytes.extend(1u32.to_be_bytes());
    bytes.extend(2u32.to_be_bytes());
    bytes.extend(8u16.to_be_bytes());
    bytes.extend(mode.to_be_bytes());
    bytes.extend([0; 12]);
    bytes.extend(compression.to_be_bytes());
    let planes = if mode == 1 {
        vec![22, 88, 255, 128]
    } else {
        vec![22, 88, 44, 99, 66, 111, 255, 128]
    };
    let raw = &planes[..channels as usize * 2];
    match compression {
        0 => bytes.extend(raw),
        1 => {
            for _ in 0..channels {
                bytes.extend(3u16.to_be_bytes());
            }
            for plane in raw.chunks_exact(2) {
                bytes.push(1);
                bytes.extend(plane);
            }
        }
        2 => {
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            z.write_all(raw).unwrap();
            bytes.extend(z.finish().unwrap());
        }
        _ => (),
    }
    bytes
}

#[test]
fn documents_use_shared_eligibility_and_full_png_preview() {
    let dir = crate::folder::tests::TempDir::new();
    for (ext, format) in [("ORA", Format::Ora), ("KRA", Format::Kra)] {
        for method in [
            zip::CompressionMethod::Stored,
            zip::CompressionMethod::Deflated,
        ] {
            let bytes = zip_entries(
                &[
                    ("preview.png", b"not the full preview"),
                    ("mergedimage.png", &png()),
                ],
                method,
            );
            let path = dir.0.join(format!("fixture.{ext}"));
            std::fs::write(&path, &bytes).unwrap();
            assert!(crate::formats::format(&path).is_some());
            let pixels = super::super::load(&path, MAX_DIMENSION).unwrap();
            assert_eq!(pixels.size, [2, 1]);
            assert_eq!(pixels.rgba, [22, 44, 66, 255, 88, 99, 111, 128]);
            assert!(archive(format, &bytes).is_ok());
        }
    }
    assert!(crate::formats::format(std::path::Path::new("a.PSD")).is_some());
    for ext in ["psb", "xcf", "zip"] {
        assert!(crate::formats::format(std::path::Path::new(&format!("a.{ext}"))).is_none());
    }
}
#[test]
fn archive_errors_are_named_and_paths_are_never_extracted() {
    for format in [Format::Ora, Format::Kra] {
        let kind = if matches!(format, Format::Ora) {
            "ORA"
        } else {
            "KRA"
        };
        assert!(
            archive(format, b"broken")
                .err()
                .unwrap()
                .to_string()
                .contains(kind)
        );
        for name in ["../outside", "/absolute", "C:/drive", "a/../bad", "a\\bad"] {
            let data = zip_entries(
                &[(name, b"bad"), ("mergedimage.png", &png())],
                zip::CompressionMethod::Stored,
            );
            assert!(
                archive(format, &data)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("unsafe ZIP entry name")
            );
        }
        let missing = zip_entries(&[("preview.png", &png())], zip::CompressionMethod::Stored);
        assert!(
            archive(format, &missing)
                .err()
                .unwrap()
                .to_string()
                .contains("missing mergedimage.png")
        );
    }
}
#[test]
fn archive_expansion_and_entry_counts_are_bounded() {
    let mut data = zip_entries(
        &[("mergedimage.png", &png())],
        zip::CompressionMethod::Deflated,
    );
    let central = data.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    data[central + 24..central + 28]
        .copy_from_slice(&((MAX_ENCODED_BYTES + 1) as u32).to_le_bytes());
    assert!(matches!(
        archive(Format::Ora, &data),
        Err(LoadError::ResourceLimit(_))
    ));
    // Actual highly compressible oversized content, not just forged metadata.
    let bomb = vec![0; MAX_ENCODED_BYTES as usize + 1];
    let data = zip_entries(
        &[("mergedimage.png", &bomb)],
        zip::CompressionMethod::Deflated,
    );
    assert!(data.len() < 100_000);
    assert!(matches!(
        archive(Format::Kra, &data),
        Err(LoadError::ResourceLimit(_))
    ));
    let mut data = zip_entries(
        &[("mergedimage.png", &png())],
        zip::CompressionMethod::Stored,
    );
    let end = data.len() - 22;
    data[end + 10..end + 12].copy_from_slice(&4097u16.to_le_bytes());
    assert!(matches!(
        archive(Format::Ora, &data),
        Err(LoadError::ResourceLimit(_))
    ));
}
#[test]
fn psd_rgb_grayscale_raw_rle_zip_and_alpha() {
    for mode in [1, 3] {
        for compression in [0, 1, 2] {
            for alpha in [false, true] {
                let channels = if mode == 1 { 1 } else { 3 } + u16::from(alpha);
                let decoded = psd(&fixture(mode, compression, channels)).unwrap();
                assert_eq!(decoded.size, [2, 1]);
                assert_eq!(
                    decoded.rgba,
                    if mode == 1 {
                        vec![22, 22, 22, 255, 88, 88, 88, if alpha { 128 } else { 255 }]
                    } else {
                        vec![22, 44, 66, 255, 88, 99, 111, if alpha { 128 } else { 255 }]
                    }
                );
            }
        }
    }
}
#[test]
fn psd_unsupported_variants_and_malformed_rows_are_specific() {
    let original = fixture(3, 0, 3);
    for (offset, value, expected) in [
        (4, 2, "PSB"),
        (22, 16, "16-bit"),
        (22, 32, "32-bit"),
        (24, 4, "CMYK"),
        (24, 9, "Lab"),
        (38, 3, "ZIP-prediction"),
    ] {
        let mut data = original.clone();
        data[offset..offset + 2].copy_from_slice(&(value as u16).to_be_bytes());
        let error = psd(&data).err().unwrap();
        assert!(matches!(error, LoadError::UnsupportedVariant(_)));
        assert!(error.to_string().contains(expected));
    }
    for n in 0..original.len() {
        assert!(psd(&original[..n]).is_err(), "truncation at {n}");
    }
    assert!(unpack_rows(&[0, 2, 254, 7], 2, 1).is_err());
    assert!(unpack_rows(&[0, 1, 0], 2, 1).is_err());
}
#[test]
fn psd_embedded_icc_and_unused_layers_follow_composite_path() {
    let mut bytes = fixture(3, 0, 3);
    let profile = moxcms::ColorProfile::new_display_p3().encode().unwrap();
    let mut resource = b"8BIM".to_vec();
    resource.extend(1039u16.to_be_bytes());
    resource.extend([0, 0]);
    resource.extend((profile.len() as u32).to_be_bytes());
    resource.extend(&profile);
    if !profile.len().is_multiple_of(2) {
        resource.push(0);
    }
    let mut replacement = (resource.len() as u32).to_be_bytes().to_vec();
    replacement.extend(&resource);
    // Arbitrary unused layer payload verifies we do not eagerly decode layers.
    replacement.extend(4u32.to_be_bytes());
    replacement.extend([255; 4]);
    bytes.splice(30..38, replacement);
    let decoded = psd(&bytes).unwrap();
    let mut expected = vec![22, 44, 66, 255, 88, 99, 111, 255];
    crate::color::normalize8(&mut expected, Some(&profile), false);
    assert_eq!(decoded.rgba, expected);
    assert!(decoded.warning.is_none());
}

#[test]
fn archive_crc_failure_and_corrupt_png_remain_named() {
    let original = png();
    let mut bytes = zip_entries(
        &[("mergedimage.png", &original)],
        zip::CompressionMethod::Stored,
    );
    let p = bytes
        .windows(original.len())
        .position(|b| b == original)
        .unwrap();
    bytes[p] ^= 1;
    let error = archive(Format::Ora, &bytes).err().unwrap();
    assert!(matches!(error, LoadError::Malformed(_)));
    assert!(error.to_string().contains("ORA"));
    let bytes = zip_entries(
        &[("mergedimage.png", b"broken")],
        zip::CompressionMethod::Stored,
    );
    assert!(
        archive(Format::Kra, &bytes)
            .err()
            .unwrap()
            .to_string()
            .contains("KRA")
    );
}

#[test]
#[ignore = "Retains original fixtures for desktop release smokes"]
fn write_document_smoke_fixtures() {
    let dir = std::env::temp_dir().join(format!("glance-document-smoke-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    for ext in ["ora", "kra"] {
        std::fs::write(
            dir.join(format!("original.{ext}")),
            zip_entries(
                &[("mergedimage.png", &png())],
                zip::CompressionMethod::Deflated,
            ),
        )
        .unwrap();
    }
    std::fs::write(dir.join("original.psd"), fixture(3, 1, 4)).unwrap();
    println!("Original document fixtures: {}", dir.display());
}
