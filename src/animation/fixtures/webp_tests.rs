use super::*;

#[derive(Clone)]
struct Part {
    size: (u32, u32),
    x: u32,
    color: [u8; 4],
    delay: u32,
    flags: u8,
}

fn animated(canvas: (u32, u32), parts: &[Part], icc: Option<&[u8]>, rotate: bool) -> Vec<u8> {
    let mut body = b"WEBP".to_vec();
    let mut extended = vec![
        2 | 16 | (u8::from(icc.is_some()) * 32) | (u8::from(rotate) * 8),
        0,
        0,
        0,
    ];
    extended.extend(&((canvas.0 - 1).to_le_bytes())[..3]);
    extended.extend(&((canvas.1 - 1).to_le_bytes())[..3]);
    riff_chunk(&mut body, b"VP8X", &extended);
    if let Some(icc) = icc {
        riff_chunk(&mut body, b"ICCP", icc);
    }
    // Nontransparent ANIM background is ignored by both composition backends.
    riff_chunk(&mut body, b"ANIM", &[80, 40, 20, 255, 3, 0]);
    for part in parts {
        let mut encoded = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut encoded)
            .write_image(
                RgbaImage::from_pixel(part.size.0, part.size.1, Rgba(part.color)).as_raw(),
                part.size.0,
                part.size.1,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        let mut frame = Vec::new();
        frame.extend(&(part.x / 2).to_le_bytes()[..3]);
        frame.extend([0; 3]);
        frame.extend(&(part.size.0 - 1).to_le_bytes()[..3]);
        frame.extend(&(part.size.1 - 1).to_le_bytes()[..3]);
        frame.extend(&part.delay.to_le_bytes()[..3]);
        frame.push(part.flags);
        frame.extend(&encoded[12..]);
        riff_chunk(&mut body, b"ANMF", &frame);
    }
    if rotate {
        let exif = [
            b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
        ];
        riff_chunk(&mut body, b"EXIF", &exif);
    }
    [
        b"RIFF".as_slice(),
        &(body.len() as u32).to_le_bytes(),
        &body,
    ]
    .concat()
}

fn parts() -> Vec<Part> {
    vec![
        Part {
            size: (4, 2),
            x: 0,
            color: [200, 0, 0, 255],
            delay: 0,
            flags: 2,
        },
        Part {
            size: (2, 2),
            x: 2,
            color: [0, 200, 0, 128],
            delay: 1,
            flags: 1,
        },
        Part {
            size: (2, 2),
            x: 0,
            color: [0, 0, 200, 255],
            delay: 27,
            flags: 2,
        },
        Part {
            size: (2, 2),
            x: 2,
            color: [100, 150, 200, 128],
            delay: 0,
            flags: 0,
        },
    ]
}

fn existing(bytes: &[u8]) -> Stream<'_> {
    let mut decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).unwrap();
    let icc = decoder.icc_profile().unwrap();
    let orientation = decoder.orientation().unwrap();
    Stream {
        loops: decoder.loop_count(),
        frames: Frames::Image(decoder.into_frames()),
        icc,
        orientation,
        warning: None,
    }
}

#[test]
fn native_partial_alpha_disposal_timing_color_and_orientation_match_existing() {
    let profile = moxcms::ColorProfile::new_display_p3().encode().unwrap();
    for (icc, rotate) in [
        (None, false),
        (Some(profile.as_slice()), true),
        (Some(b"bad ICC".as_slice()), true),
    ] {
        let bytes = animated((4, 2), &parts(), icc, rotate);
        let prepared = prepare(Format::Standard(ImageFormat::WebP), &bytes).unwrap();
        assert!(matches!(prepared, std::borrow::Cow::Owned(_)));
        let mut native = Stream::open(Format::Standard(ImageFormat::WebP), &prepared)
            .unwrap()
            .unwrap();
        assert!(matches!(native.frames, Frames::WebP(_)));
        assert!(matches!(native.loops, LoopCount::Finite(n) if n.get() == 3));
        let mut reference = existing(&bytes);
        for part in parts() {
            let actual = native.next().unwrap().unwrap();
            let expected = reference.next().unwrap().unwrap();
            assert_eq!(actual.size, expected.size);
            assert_eq!(actual.rgba, expected.rgba);
            assert_eq!(actual.warning, expected.warning);
            assert_eq!(
                actual.animation_delay,
                Some(Duration::from_millis(u64::from(part.delay)))
            );
            assert_eq!(actual.animation_delay, expected.animation_delay);
        }
        assert!(native.next().unwrap().is_none());
        assert!(reference.next().unwrap().is_none());
    }
}

#[test]
fn large_canvas_and_long_duration_use_existing_backend() {
    let small = Part {
        size: (1, 1),
        x: 0,
        color: [255, 0, 0, 255],
        delay: 0x00ff_ffff,
        flags: 2,
    };
    for (canvas, parts) in [
        ((3000, 3000), vec![small.clone(); 2]),
        ((1, 1), vec![small; 129]),
    ] {
        let bytes = animated(canvas, &parts, None, false);
        let prepared = prepare(Format::Standard(ImageFormat::WebP), &bytes).unwrap();
        assert!(matches!(prepared, std::borrow::Cow::Borrowed(_)));
        let mut stream = Stream::open(Format::Standard(ImageFormat::WebP), &prepared)
            .unwrap()
            .unwrap();
        assert!(matches!(stream.frames, Frames::Image(_)));
        // Compare the actual decoder output without building large minification levels.
        let Frames::Image(frames) = &mut stream.frames else {
            unreachable!()
        };
        for expected in &parts {
            let frame = frames.next().unwrap().unwrap();
            assert_eq!(frame.buffer().dimensions(), canvas);
            assert_eq!(frame.buffer().get_pixel(0, 0).0, expected.color);
            assert_eq!(frame.delay().numer_denom_ms(), (expected.delay, 1));
        }
        assert!(frames.next().is_none());
    }
}

#[test]
fn webp_static_and_resource_admission_stay_unchanged() {
    let mut bytes = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
        .write_image(&[12, 34, 56, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    assert!(
        Stream::open(Format::Standard(ImageFormat::WebP), &bytes)
            .unwrap()
            .is_none()
    );
    let bytes = animated((4096, 4096), &parts(), None, false);
    assert!(matches!(
        Stream::open(Format::Standard(ImageFormat::WebP), &bytes),
        Err(LoadError::ResourceLimit(_))
    ));
}

#[test]
fn corrupt_native_later_frame_is_error_not_eof() {
    let mut bytes = animated((4, 2), &parts(), None, false);
    let mut at = 12;
    let mut seen = 0;
    while at < bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        if &bytes[at..at + 4] == b"ANMF" {
            seen += 1;
            if seen == 2 {
                // Keep VP8L signature/dimensions; destroy only compressed image data.
                bytes[at + 8 + 16 + 8 + 5..at + 8 + size].fill(0xff);
                break;
            }
        }
        at += 8 + size + (size & 1);
    }
    let mut stream = Stream::open(Format::Standard(ImageFormat::WebP), &bytes)
        .unwrap()
        .unwrap();
    assert!(matches!(stream.frames, Frames::WebP(_)));
    assert!(stream.next().unwrap().is_some());
    assert!(matches!(stream.next(), Err(LoadError::Malformed(_))));
}

#[test]
fn worker_preserves_disposal_compatibility_and_original_file() {
    let bytes = animated((4, 2), &parts(), None, false);
    let dir = TempDir::new();
    let w = worker(&dir, "webp", &bytes);
    let mut reference = existing(&bytes);
    let expected = reference.next().unwrap().unwrap();
    let first = crate::decode::load(&dir.0.join("animation.webp"), 8192).unwrap();
    assert_eq!(first.rgba, expected.rgba);
    for _ in 1..parts().len() {
        w.advance(None);
        let actual = wait(&w).unwrap().unwrap().pixels;
        let expected = reference.next().unwrap().unwrap();
        assert_eq!(actual.rgba, expected.rgba);
        assert_eq!(actual.animation_delay, expected.animation_delay);
    }
    assert_eq!(std::fs::read(dir.0.join("animation.webp")).unwrap(), bytes);
}

#[test]
fn native_exact_count_detects_both_early_end_and_extra_output() {
    let bytes = webp(0);
    for count in [2, 4] {
        let mut preflight = super::super::webp::Preflight::read(&bytes, (16, 16)).unwrap();
        preflight.frames = count;
        let mut native = super::super::webp::Native::new(&bytes, (16, 16), preflight).unwrap();
        for _ in 0..count.min(3) {
            assert!(native.next().unwrap().is_some());
        }
        assert!(native.next().is_err());
    }
}

#[test]
fn native_cumulative_timestamp_at_signed_limit_preserves_every_delay() {
    let part = Part {
        size: (1, 1),
        x: 0,
        color: [255, 0, 0, 255],
        delay: 0x00ff_ffff,
        flags: 2,
    };
    let mut parts = vec![part; 129];
    parts[128].delay = 127;
    let bytes = animated((1, 1), &parts, None, false);
    let prepared = prepare(Format::Standard(ImageFormat::WebP), &bytes).unwrap();
    let mut stream = Stream::open(Format::Standard(ImageFormat::WebP), &prepared)
        .unwrap()
        .unwrap();
    assert!(matches!(stream.frames, Frames::WebP(_)));
    let mut total = Duration::ZERO;
    for part in parts {
        let frame = stream.next().unwrap().unwrap();
        let delay = frame.animation_delay.unwrap();
        assert_eq!(delay, Duration::from_millis(u64::from(part.delay)));
        total += delay;
    }
    assert_eq!(total, Duration::from_millis(i32::MAX as u64));
    assert!(stream.next().unwrap().is_none());
}

#[test]
fn webp_preflight_rejects_truncation_and_canvas_disagreement() {
    let bytes = animated((4, 2), &parts(), None, false);
    for cut in 0..bytes.len() {
        assert!(super::super::webp::Preflight::read(&bytes[..cut], (4, 2)).is_err());
    }
    assert!(super::super::webp::Preflight::read(&bytes, (2, 4)).is_err());
}
