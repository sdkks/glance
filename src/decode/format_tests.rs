use super::*;
use crate::folder::tests::TempDir;
use image::{ExtendedColorType, ImageEncoder};

fn png(size: u32, color: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            &color.repeat((size * size) as usize),
            size,
            size,
            ExtendedColorType::Rgba8,
        )
        .unwrap();
    bytes
}

fn icon(cursor: bool) -> Vec<u8> {
    let images = [png(1, [1, 2, 3, 255]), png(2, [22, 44, 66, 255])];
    let mut bytes = vec![0, 0, if cursor { 2 } else { 1 }, 0, 2, 0];
    let mut offset = 38u32;
    for (size, data) in [1u8, 2].into_iter().zip(&images) {
        bytes.extend([size, size, 0, 0]);
        // CUR hotspots intentionally exceed legal ICO bit depths.
        bytes.extend(if cursor {
            [0, 0, 200, 0]
        } else {
            [1, 0, 32, 0]
        });
        bytes.extend((data.len() as u32).to_le_bytes());
        bytes.extend(offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for data in images {
        bytes.extend(data);
    }
    bytes
}

fn tga(rle: bool, top: bool) -> Vec<u8> {
    let mut data = vec![0; 18];
    data[2] = if rle { 10 } else { 2 };
    data[12] = 2;
    data[14] = 2;
    data[16] = 24;
    data[17] = if top { 32 } else { 0 };
    // Two rows with distinct colors; RLE packets stop at row boundaries.
    let rows = if top {
        [[66, 44, 22], [3, 2, 1]]
    } else {
        [[3, 2, 1], [66, 44, 22]]
    };
    for row in rows {
        if rle {
            data.push(0x81);
            data.extend(row);
        } else {
            data.extend(row);
            data.extend(row);
        }
    }
    data
}

fn pcx() -> Vec<u8> {
    let mut data = vec![0; 128];
    data[..4].copy_from_slice(&[10, 5, 1, 8]);
    data[8] = 1; // xmax=1 -> two pixels
    data[65] = 3;
    data[66] = 2; // even bytes per plane
    data[68] = 1;
    data.extend([22, 22, 44, 44, 66, 66]);
    data
}

fn sgi(depth: u8, channels: u16) -> Vec<u8> {
    let mut data = vec![0; 512];
    data[..6].copy_from_slice(&[1, 218, 0, depth, 0, 3]);
    data[6..12].copy_from_slice(&[0, 2, 0, 1, 0, channels as u8]);
    data[16..20].copy_from_slice(&65535u32.to_be_bytes());
    for value in [22u16, 44, 66, 255].into_iter().take(channels as usize) {
        for _ in 0..2 {
            if depth == 2 {
                data.extend((value * 257).to_be_bytes());
            } else {
                data.push(value as u8);
            }
        }
    }
    data
}

fn gif(animated: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
        let frame =
            |color| image::Frame::new(image::RgbaImage::from_pixel(2, 1, image::Rgba(color)));
        encoder.encode_frame(frame([22, 44, 66, 255])).unwrap();
        if animated {
            encoder.encode_frame(frame([200, 100, 50, 255])).unwrap();
        }
    }
    bytes
}

fn webp(color: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
        .write_image(&color.repeat(2), 2, 1, ExtendedColorType::Rgba8)
        .unwrap();
    bytes
}

fn riff_chunk(data: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
    data.extend(kind);
    data.extend((payload.len() as u32).to_le_bytes());
    data.extend(payload);
    if payload.len() % 2 == 1 {
        data.push(0);
    }
}

// Independently assembled WebP animation container around original lossless
// frames. Canvas 2x1; full-canvas replace frames, 100ms each, no disposal.
fn animated_webp() -> Vec<u8> {
    let mut data = b"WEBP".to_vec();
    riff_chunk(&mut data, b"VP8X", &[2, 0, 0, 0, 1, 0, 0, 0, 0, 0]);
    riff_chunk(&mut data, b"ANIM", &[0; 6]);
    for color in [[22, 44, 66, 255], [200, 100, 50, 255]] {
        let static_frame = webp(color);
        let mut frame = vec![0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 100, 0, 0, 2];
        frame.extend(&static_frame[12..]);
        riff_chunk(&mut data, b"ANMF", &frame);
    }
    let mut result = b"RIFF".to_vec();
    result.extend((data.len() as u32).to_le_bytes());
    result.extend(data);
    result
}

fn icns() -> Vec<u8> {
    let payload = png(16, [22, 44, 66, 255]);
    let mut data = b"icns".to_vec();
    data.extend((16 + payload.len() as u32).to_be_bytes());
    data.extend(b"icp4");
    data.extend((8 + payload.len() as u32).to_be_bytes());
    data.extend(payload);
    data
}

type Fixture = (&'static str, Vec<u8>, [usize; 2], [u8; 4]);

fn fixtures() -> Vec<Fixture> {
    let mut bmp = Vec::new();
    image::codecs::bmp::BmpEncoder::new(&mut bmp)
        .write_image(&[22, 44, 66, 22, 44, 66], 2, 1, ExtendedColorType::Rgb8)
        .unwrap();
    vec![
        ("bmp", bmp, [2, 1], [22, 44, 66, 255]),
        ("ico", icon(false), [2, 2], [22, 44, 66, 255]),
        ("cur", icon(true), [2, 2], [22, 44, 66, 255]),
        ("icns", icns(), [16, 16], [22, 44, 66, 255]),
        ("tga", tga(false, true), [2, 2], [22, 44, 66, 255]),
        ("pgm", b"P5\n2 1\n255\n\x16\x16".to_vec(), [2, 1], [22, 22, 22, 255]),
        ("ppm", b"P6\n2 1\n255\n\x16\x2c\x42\x16\x2c\x42".to_vec(), [2, 1], [22, 44, 66, 255]),
        ("pbm", b"P4\n2 1\n\xc0".to_vec(), [2, 1], [0, 0, 0, 255]),
        ("pam", b"P7\nWIDTH 2\nHEIGHT 1\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n\x16\x2c\x42\xff\x16\x2c\x42\xff".to_vec(), [2, 1], [22, 44, 66, 255]),
        ("wbmp", vec![0, 0, 2, 1, 0xc0], [2, 1], [255, 255, 255, 255]),
        ("xbm", b"#define test_width 2\n#define test_height 1\nstatic unsigned char test_bits[] = { 0x03 };\n".to_vec(), [2, 1], [0, 0, 0, 255]),
        ("xpm", b"/* XPM */\nstatic char *test[] = {\n\"2 1 1 1\",\n\"a c #162c42\",\n\"aa\"\n};\n".to_vec(), [2, 1], [22, 44, 66, 255]),
        ("pcx", pcx(), [2, 1], [22, 44, 66, 255]),
        ("rgb", sgi(1, 3), [2, 1], [22, 44, 66, 255]),
        ("gif", gif(false), [2, 1], [22, 44, 66, 255]),
        ("webp", webp([22, 44, 66, 255]), [2, 1], [22, 44, 66, 255]),
    ]
}

#[test]
fn everyday_formats_decode_dimensions_and_pixels() {
    let dir = TempDir::new();
    for (ext, bytes, size, color) in fixtures() {
        let path = dir.0.join(format!("fixture.{ext}"));
        std::fs::write(&path, bytes).unwrap();
        let pixels = load(&path, MAX_DIMENSION).unwrap_or_else(|e| panic!("{ext}: {e}"));
        assert_eq!(pixels.size, size, "{ext}");
        assert_eq!(pixels.clipboard_pixel(0), color, "{ext}");
        assert!(pixels.warning.is_none(), "{ext}");
        assert!(!pixels.levels.is_empty(), "{ext}");
    }
}

#[test]
fn tga_raw_rle_and_vertical_orientation() {
    let dir = TempDir::new();
    for rle in [false, true] {
        for top in [false, true] {
            let path = dir.0.join("fixture.tga");
            std::fs::write(&path, tga(rle, top)).unwrap();
            let pixels = load(&path, MAX_DIMENSION).unwrap();
            assert_eq!(pixels.clipboard_pixel(0), [22, 44, 66, 255]);
            assert_eq!(pixels.clipboard_pixel(2), [1, 2, 3, 255]);
        }
    }
}

#[test]
fn animations_open_first_frame_without_playback() {
    let dir = TempDir::new();
    for (ext, data) in [("gif", gif(true)), ("webp", animated_webp())] {
        let path = dir.0.join(format!("animation.{ext}"));
        std::fs::write(&path, data).unwrap();
        let pixels = load(&path, MAX_DIMENSION).unwrap();
        assert_eq!(pixels.size, [2, 1]);
        assert_eq!(pixels.clipboard_pixel(0), [22, 44, 66, 255]);
    }
}

#[test]
fn new_formats_retain_available_16bit_samples() {
    let dir = TempDir::new();
    for (ext, data) in [
        ("pgm", b"P5\n2 1\n65535\n\x12\x34\xab\xcd".to_vec()),
        ("rgb", sgi(2, 3)),
        ("xpm", b"/* XPM */\nstatic char *test[] = {\n\"1 1 1 1\",\n\"a c #123456789abc\",\n\"a\"\n};\n".to_vec()),
    ] {
        let path = dir.0.join(format!("deep.{ext}"));
        std::fs::write(&path, data).unwrap();
        let pixels = load(&path, MAX_DIMENSION).unwrap();
        let samples = pixels.rgba16.unwrap();
        assert!(pixels.rgba.is_empty());
        assert!(pixels.levels[0].half);
        match ext {
            "pgm" => assert_eq!(samples, [0x1234, 0x1234, 0x1234, 65535, 0xabcd, 0xabcd, 0xabcd, 65535]),
            "rgb" => assert_eq!(&samples[..4], &[22 * 257, 44 * 257, 66 * 257, 65535]),
            _ => assert_eq!(samples, [0x1234, 0x5678, 0x9abc, 65535]),
        }
    }
}

#[test]
fn each_format_rejects_malformed_or_truncated_input_without_unwinding() {
    let dir = TempDir::new();
    for (ext, data, _, _) in fixtures() {
        let path = dir.0.join(format!("bad.{ext}"));
        for bytes in [&b"broken"[..], &data[..data.len() / 2]] {
            std::fs::write(&path, bytes).unwrap();
            let result = std::panic::catch_unwind(|| load(&path, MAX_DIMENSION));
            let error = result
                .unwrap_or_else(|_| panic!("escaped panic: {ext}"))
                .err()
                .unwrap_or_else(|| panic!("malformed accepted: {ext}"));
            assert!(
                matches!(
                    error,
                    LoadError::Io(_)
                        | LoadError::Malformed(_)
                        | LoadError::UnsupportedVariant(_)
                        | LoadError::ResourceLimit(_)
                ),
                "{ext}: {error}"
            );
            assert!(!error.to_string().is_empty());
        }
    }
}

#[test]
fn unsupported_wbmp_variant_is_named_and_next_load_works() {
    let dir = TempDir::new();
    let path = dir.0.join("variant.wbmp");
    std::fs::write(&path, [1, 0, 1, 1, 0]).unwrap();
    let error = load(&path, MAX_DIMENSION).err().unwrap();
    assert!(matches!(error, LoadError::UnsupportedVariant(_)));
    assert!(error.to_string().contains("type 1"));
    std::fs::write(&path, [0, 0, 1, 1, 0]).unwrap();
    assert!(load(&path, MAX_DIMENSION).is_ok());
}

#[test]
fn eligibility_is_exact_case_insensitive_dispatch_and_hidden_rules_unchanged() {
    for ext in [
        "jpg", "jpeg", "png", "tif", "tiff", "bmp", "ico", "cur", "icns", "tga", "pnm", "pbm",
        "pgm", "ppm", "pam", "wbmp", "xbm", "xpm", "pcx", "sgi", "bw", "rgb", "rgba", "gif",
        "webp", "jxl", "svg", "svgz",
    ] {
        assert!(crate::folder::eligible(Path::new(&format!(
            "file.{}",
            ext.to_uppercase()
        ))));
    }
    for name in [
        "no-extension",
        "file.avif",
        "file.xcf",
        "file.dds",
        "file.ora",
        "file.otb",
        "file.bm",
        "file.iris",
    ] {
        assert!(!crate::folder::eligible(Path::new(name)), "{name}");
    }
    let dir = TempDir::new();
    std::fs::write(dir.0.join(".hidden.gif"), gif(false)).unwrap();
    std::fs::write(dir.0.join("visible.WebP"), webp([1, 2, 3, 255])).unwrap();
    let folder = crate::folder::Folder::open(dir.0.clone()).unwrap();
    assert_eq!(folder.files, [dir.0.join("visible.WebP")]);
}

#[test]
fn cursor_directory_rejects_invalid_ranges_and_uses_first_size_tie() {
    let dir = TempDir::new();
    let path = dir.0.join("directory.cur");
    for mutation in [0, 1, 2, 3] {
        let mut data = icon(true);
        match mutation {
            0 => data[4..6].copy_from_slice(&0u16.to_le_bytes()),
            1 => data[18..22].copy_from_slice(&u32::MAX.to_le_bytes()),
            2 => data[14..18].copy_from_slice(&u32::MAX.to_le_bytes()),
            _ => data[18..22].copy_from_slice(&6u32.to_le_bytes()),
        }
        std::fs::write(&path, data).unwrap();
        assert!(matches!(
            load(&path, MAX_DIMENSION),
            Err(LoadError::Malformed(_))
        ));
    }
    // Reverse two same-size payloads: first directory entry wins area ties.
    let red = png(2, [200, 0, 0, 255]);
    let green = png(2, [0, 200, 0, 255]);
    let mut data = vec![0, 0, 2, 0, 2, 0];
    let mut offset = 38u32;
    for payload in [&red, &green] {
        data.extend([2, 2, 0, 0, 1, 0, 1, 0]);
        data.extend((payload.len() as u32).to_le_bytes());
        data.extend(offset.to_le_bytes());
        offset += payload.len() as u32;
    }
    data.extend(red);
    data.extend(green);
    std::fs::write(&path, data).unwrap();
    assert_eq!(
        load(&path, MAX_DIMENSION).unwrap().clipboard_pixel(0),
        [200, 0, 0, 255]
    );
}

#[test]
fn sgi_channel_aliases_and_resource_limit() {
    let dir = TempDir::new();
    for (ext, channels, expected) in [
        ("bw", 1, [22, 22, 22, 255]),
        ("rgb", 3, [22, 44, 66, 255]),
        ("rgba", 4, [22, 44, 66, 255]),
    ] {
        let path = dir.0.join(format!("channels.{ext}"));
        std::fs::write(&path, sgi(1, channels)).unwrap();
        assert_eq!(
            load(&path, MAX_DIMENSION).unwrap().clipboard_pixel(0),
            expected
        );
    }
    let path = dir.0.join("oversize.sgi");
    let mut data = sgi(2, 4);
    data[6..8].copy_from_slice(&16385u16.to_be_bytes());
    std::fs::write(&path, data).unwrap();
    assert!(matches!(
        load(&path, MAX_DIMENSION),
        Err(LoadError::ResourceLimit(_))
    ));
}

#[test]
fn webp_icc_and_orientation_use_existing_normalization_seam() {
    let dir = TempDir::new();
    let path = dir.0.join("tagged.webp");
    let static_frame = webp([153, 102, 51, 255]);
    let mut body = b"WEBP".to_vec();
    // ICC + EXIF flags, 2x1 canvas.
    riff_chunk(&mut body, b"VP8X", &[0x28, 0, 0, 0, 1, 0, 0, 0, 0, 0]);
    riff_chunk(
        &mut body,
        b"ICCP",
        &moxcms::ColorProfile::new_display_p3().encode().unwrap(),
    );
    body.extend(&static_frame[12..]);
    // Original little-endian TIFF EXIF, one SHORT orientation tag = 6.
    let exif = [
        b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
    ];
    riff_chunk(&mut body, b"EXIF", &exif);
    let mut encoded = b"RIFF".to_vec();
    encoded.extend((body.len() as u32).to_le_bytes());
    encoded.extend(body);
    std::fs::write(&path, encoded).unwrap();
    let pixels = load(&path, MAX_DIMENSION).unwrap();
    assert_eq!(pixels.size, [1, 2]);
    assert!(pixels.warning.is_none());
    for (actual, expected) in pixels
        .clipboard_pixel(0)
        .into_iter()
        .zip([162u8, 99, 39, 255])
    {
        assert!(actual.abs_diff(expected) <= 2);
    }
}

#[test]
fn dib_cursor_and_horizontal_tga_origin() {
    let dir = TempDir::new();
    let path = dir.0.join("dib.cur");
    let mut dib = vec![0; 48];
    dib[..4].copy_from_slice(&40u32.to_le_bytes());
    dib[4..8].copy_from_slice(&1u32.to_le_bytes());
    dib[8..12].copy_from_slice(&2u32.to_le_bytes()); // XOR image + AND mask height
    dib[12..16].copy_from_slice(&[1, 0, 32, 0]);
    dib[40..44].copy_from_slice(&[66, 44, 22, 255]);
    let mut cur = vec![0, 0, 2, 0, 1, 0, 1, 1, 0, 0, 0, 0, 100, 0];
    cur.extend((dib.len() as u32).to_le_bytes());
    cur.extend(22u32.to_le_bytes());
    cur.extend(dib);
    std::fs::write(&path, cur).unwrap();
    assert_eq!(
        load(&path, MAX_DIMENSION).unwrap().clipboard_pixel(0),
        [22, 44, 66, 255]
    );
    let path = dir.0.join("right-origin.tga");
    let mut data = vec![0; 18];
    data[2] = 2;
    data[12] = 2;
    data[14] = 1;
    data[16] = 24;
    data[17] = 0x30; // top-right origin
    data.extend([3, 2, 1, 66, 44, 22]);
    std::fs::write(&path, data).unwrap();
    let pixels = load(&path, MAX_DIMENSION).unwrap();
    assert_eq!(pixels.clipboard_pixel(0), [22, 44, 66, 255]);
    assert_eq!(pixels.clipboard_pixel(1), [1, 2, 3, 255]);
}

#[test]
fn pnm_ascii_and_xpm_transparency() {
    let dir = TempDir::new();
    for (ext, bytes, color) in [
        ("pbm", &b"P1\n# comment\n1 1\n1\n"[..], [0, 0, 0, 255]),
        ("pgm", &b"P2\n1 1\n255\n22\n"[..], [22, 22, 22, 255]),
        ("ppm", &b"P3\n1 1\n255\n22 44 66\n"[..], [22, 44, 66, 255]),
        (
            "xpm",
            &b"/* XPM */\nstatic char *test[] = {\n\"1 1 1 1\",\n\"a c None\",\n\"a\"\n};\n"[..],
            [0, 0, 0, 0],
        ),
    ] {
        let path = dir.0.join(format!("fixture.{ext}"));
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(
            load(&path, MAX_DIMENSION).unwrap().clipboard_pixel(0),
            color
        );
    }
}

#[test]
fn unsupported_icns_jpeg2000_is_named() {
    let dir = TempDir::new();
    let path = dir.0.join("jpeg2000.icns");
    let mut data = b"icns".to_vec();
    data.extend(28u32.to_be_bytes());
    data.extend(b"icp4");
    data.extend(20u32.to_be_bytes());
    data.extend([0, 0, 0, 12, b'j', b'P', b' ', b' ', 13, 10, 0x87, 10]);
    std::fs::write(&path, data).unwrap();
    let error = load(&path, MAX_DIMENSION).err().unwrap();
    assert!(matches!(error, LoadError::UnsupportedVariant(_)));
    assert!(error.to_string().contains("Jpeg 2000"));
}

#[test]
#[ignore = "Creates retained original fixtures for release launch smokes"]
fn write_format_smoke_fixtures() {
    let dir = std::env::var_os("GLANCE_FORMAT_SMOKE_DIR")
        .map(std::path::PathBuf::from)
        .expect("provide a fresh mktemp directory");
    for (ext, data, _, _) in fixtures() {
        std::fs::write(dir.join(format!("original.{ext}")), data).unwrap();
    }
    std::fs::write(dir.join("animated.gif"), gif(true)).unwrap();
    std::fs::write(dir.join("animated.webp"), animated_webp()).unwrap();
    println!("Retained format fixtures: {}", dir.display());
}
