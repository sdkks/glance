use super::*;
use crate::{
    folder::{FileVersion, tests::TempDir},
    formats::Format,
};
use image::{ImageEncoder, Rgba, RgbaImage};
use std::io::Cursor;

fn gif(repeat: Option<image::codecs::gif::Repeat>) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
        if let Some(repeat) = repeat {
            encoder.set_repeat(repeat).unwrap();
        }
        for color in [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]] {
            encoder
                .encode_frame(image::Frame::from_parts(
                    RgbaImage::from_pixel(16, 16, Rgba(color)),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(30, 1),
                ))
                .unwrap();
        }
    }
    bytes
}

fn png(color: [u8; 4], size: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            RgbaImage::from_pixel(size, size, Rgba(color)).as_raw(),
            size,
            size,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    bytes
}
fn png_chunks(bytes: &[u8]) -> Vec<(&[u8], &[u8])> {
    let mut out = Vec::new();
    let mut at = 8;
    while at < bytes.len() {
        let n = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        out.push((&bytes[at + 4..at + 8], &bytes[at + 8..at + 8 + n]));
        at += n + 12;
    }
    out
}
fn chunk(out: &mut Vec<u8>, name: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    out.extend(name);
    out.extend(data);
    let mut crc = !0u32;
    for b in name.iter().chain(data) {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    out.extend((!crc).to_be_bytes());
}
fn apng(plays: u32) -> Vec<u8> {
    let first = png([255, 0, 0, 255], 16);
    let second = png([0, 255, 0, 255], 8);
    let mut out = first[..8].to_vec();
    chunk(&mut out, b"IHDR", png_chunks(&first)[0].1);
    chunk(
        &mut out,
        b"acTL",
        &[2u32.to_be_bytes(), plays.to_be_bytes()].concat(),
    );
    let control = |seq: u32, size: u32, x: u32| {
        [
            seq.to_be_bytes().to_vec(),
            size.to_be_bytes().to_vec(),
            size.to_be_bytes().to_vec(),
            x.to_be_bytes().to_vec(),
            0u32.to_be_bytes().to_vec(),
            30u16.to_be_bytes().to_vec(),
            1000u16.to_be_bytes().to_vec(),
            vec![0, 0],
        ]
        .concat()
    };
    chunk(&mut out, b"fcTL", &control(0, 16, 0));
    for (name, data) in png_chunks(&first) {
        if name == b"IDAT" {
            chunk(&mut out, b"IDAT", data);
        }
    }
    chunk(&mut out, b"fcTL", &control(1, 8, 8));
    let mut seq = 2u32;
    for (name, data) in png_chunks(&second) {
        if name == b"IDAT" {
            chunk(
                &mut out,
                b"fdAT",
                &[seq.to_be_bytes().as_slice(), data].concat(),
            );
            seq += 1;
        }
    }
    chunk(&mut out, b"IEND", &[]);
    out
}
fn riff_chunk(out: &mut Vec<u8>, name: &[u8; 4], bytes: &[u8]) {
    out.extend(name);
    out.extend((bytes.len() as u32).to_le_bytes());
    out.extend(bytes);
    if bytes.len() % 2 == 1 {
        out.push(0);
    }
}
fn webp(plays: u16) -> Vec<u8> {
    let mut out = b"WEBP".to_vec();
    let mut extended = vec![2, 0, 0, 0];
    extended.extend([15, 0, 0, 15, 0, 0]);
    riff_chunk(&mut out, b"VP8X", &extended);
    riff_chunk(
        &mut out,
        b"ANIM",
        &[vec![0; 4], plays.to_le_bytes().to_vec()].concat(),
    );
    for color in [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]] {
        let mut encoded = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut encoded)
            .write_image(
                RgbaImage::from_pixel(16, 16, Rgba(color)).as_raw(),
                16,
                16,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        let mut frame = vec![0; 6]; // x/y
        frame.extend([15, 0, 0, 15, 0, 0, 30, 0, 0, 2]); // size, duration, no blend
        frame.extend_from_slice(&encoded[12..]);
        riff_chunk(&mut out, b"ANMF", &frame);
    }
    [b"RIFF".as_slice(), &(out.len() as u32).to_le_bytes(), &out].concat()
}
fn wait(worker: &Worker) -> Result<Option<Frame>, LoadError> {
    let end = std::time::Instant::now() + Duration::from_secs(5);
    let mut state = worker.shared.state.lock().unwrap();
    while state.result.is_none() {
        let left = end.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            drop(state);
            panic!("animation worker timed out");
        }
        state = worker.shared.wake.wait_timeout(state, left).unwrap().0;
    }
    state.result.take().unwrap()
}
fn worker(dir: &TempDir, extension: &str, bytes: &[u8]) -> Worker {
    let path = dir.0.join(format!("animation.{extension}"));
    std::fs::write(&path, bytes).unwrap();
    Worker::new(
        FileVersion::read(&path).unwrap(),
        eframe::egui::Context::default(),
    )
    .unwrap()
}

#[test]
fn gif_absent_infinite_and_finite_metadata_are_distinct() {
    for (repeat, plays) in [
        (None, Some(1)),
        (Some(image::codecs::gif::Repeat::Finite(2)), Some(3)),
        (Some(image::codecs::gif::Repeat::Infinite), None),
    ] {
        let bytes = gif(repeat);
        let loops = gif_loops(&bytes).unwrap();
        assert_eq!(
            match loops {
                LoopCount::Infinite => None,
                LoopCount::Finite(n) => Some(n.get()),
            },
            plays
        );
        // An identifier in a comment is not a loop instruction.
        let mut comment = bytes.clone();
        comment.splice(13..13, b"\x21\xfe\x0bNETSCAPE2.0\x00".iter().copied());
        // Real files with a global table need the comment inserted after it.
        // The parser must reject this malformed placement, not scan for strings.
        if bytes[10] & 0x80 != 0 {
            assert!(gif_loops(&comment).is_err());
        }
    }
}
#[test]
fn codecs_composition_timing_and_play_counts() {
    for (format, bytes, frames, plays) in [
        (ImageFormat::Gif, gif(None), 3, 1),
        (ImageFormat::Png, apng(2), 2, 2),
        (ImageFormat::WebP, webp(3), 3, 3),
    ] {
        let mut stream = Stream::open(Format::Standard(format), bytes.into())
            .unwrap()
            .unwrap();
        assert!(matches!(stream.loops,LoopCount::Finite(n) if n.get()==plays));
        let first = stream.next().unwrap().unwrap();
        assert_eq!(first.animation_delay, Some(Duration::from_millis(30)));
        assert_eq!(&first.rgba[..4], &[255, 0, 0, 255]);
        let second = stream.next().unwrap().unwrap();
        if format == ImageFormat::Png {
            assert_eq!(&second.rgba[..4], &[255, 0, 0, 255]);
            assert_eq!(&second.rgba[8 * 4..9 * 4], &[0, 255, 0, 255]);
        } else {
            assert_eq!(&second.rgba[..4], &[0, 255, 0, 255]);
        }
        let mut count = 2;
        while stream.next().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, frames);
    }
}
#[test]
fn gif_encoded_centisecond_timing_matrix() {
    let matrix = [(0, 100), (1, 100), (2, 20), (5, 50), (10, 100), (20, 200)];
    let mut bytes = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
        for (cs, _) in matrix {
            encoder
                .encode_frame(image::Frame::from_parts(
                    RgbaImage::from_pixel(16, 16, Rgba([255, 0, 0, 255])),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(cs * 10, 1),
                ))
                .unwrap();
        }
    }
    let mut stream = Stream::open(Format::Standard(ImageFormat::Gif), bytes.into())
        .unwrap()
        .unwrap();
    for (cs, ms) in matrix {
        let frame = stream.next().unwrap().unwrap();
        assert_eq!(
            frame.animation_delay,
            Some(Duration::from_millis(ms)),
            "{cs} cs"
        );
    }
    assert!(stream.next().unwrap().is_none());
}

#[test]
fn prefetch_load_is_first_frame_only() {
    let dir = TempDir::new();
    for (ext, bytes) in [("gif", gif(None)), ("png", apng(0)), ("webp", webp(0))] {
        let path = dir.0.join(format!("first.{ext}"));
        std::fs::write(&path, bytes).unwrap();
        let pixels = crate::decode::load(&path, 8192).unwrap();
        assert_eq!(&pixels.rgba[..4], &[255, 0, 0, 255]);
        assert_eq!(pixels.animation_delay, Some(Duration::from_millis(30)));
        // Pixels contain timing only, not a worker or a collection of frames.
        assert_eq!(pixels.size, [16, 16]);
    }
}
#[test]
fn worker_finite_stops_manual_wraps_and_repeated_loops_release_frames() {
    let dir = TempDir::new();
    for (ext, bytes, frames, plays) in [
        ("gif", gif(None), 3, 1),
        ("png", apng(2), 2, 2),
        ("webp", webp(3), 3, 3),
    ] {
        let w = worker(&dir, ext, &bytes);
        for index in 1..frames * plays {
            w.advance(None);
            let frame = wait(&w).unwrap().unwrap();
            assert_eq!(frame.final_frame, index == frames * plays - 1);
        }
        w.advance(None);
        assert!(wait(&w).unwrap().is_none());
        w.advance(Some(frames - 1));
        assert_eq!(
            &wait(&w).unwrap().unwrap().pixels.rgba[..4],
            &[255, 0, 0, 255]
        );
    }
    let w = worker(
        &dir,
        "gif",
        &gif(Some(image::codecs::gif::Repeat::Infinite)),
    );
    let mut weak = Vec::new();
    let mut displayed = None;
    let mut clock = Clock::new(Duration::ZERO, Duration::from_millis(30));
    for n in 0..300 {
        w.advance(None);
        let frame = wait(&w).unwrap().unwrap();
        assert!(!frame.final_frame);
        clock.advance(frame.pixels.animation_delay.unwrap());
        weak.push(Arc::downgrade(&frame.pixels));
        let live_delivered = weak.iter().filter(|w| w.strong_count() > 0).count();
        let lookahead = w
            .shared
            .state
            .lock()
            .unwrap()
            .lookahead
            .as_ref()
            .and_then(std::sync::Weak::upgrade);
        let resident_count = live_delivered + usize::from(lookahead.is_some());
        assert!(
            resident_count <= 3,
            "displayed + candidate + lookahead: {resident_count}"
        );
        displayed = Some(frame.pixels);
        // One displayed Arc; all previously delivered frames have been released.
        // The test-only weak probe measures the worker's actual lookahead too.
        assert_eq!(weak.iter().filter(|w| w.strong_count() > 0).count(), 1);
        assert!(w.shared.state.lock().unwrap().result.is_none());
        assert_eq!(clock.deadline, Duration::from_millis(30 * (n + 2)));
    }
    assert!(displayed.is_some());
}
#[test]
fn truncation_version_change_and_static_detection() {
    let dir = TempDir::new();
    let bytes = gif(None);
    for cut in 0..bytes.len() {
        let result = std::panic::catch_unwind(|| {
            Stream::open(Format::Standard(ImageFormat::Gif), Arc::from(&bytes[..cut]))
        });
        assert!(result.is_ok());
        assert!(result.unwrap().is_err());
    }
    assert!(
        Stream::open(Format::Standard(ImageFormat::Png), png([0; 4], 16).into())
            .unwrap()
            .is_none()
    );
    let w = worker(&dir, "gif", &bytes);
    std::fs::write(dir.0.join("animation.gif"), b"changed").unwrap();
    w.advance(None);
    assert!(wait(&w).is_err());
    let mut decoded = Cursor::new(webp(0));
    // The generated WebP is independently understood by the static decoder too.
    assert_eq!(
        image::load(&mut decoded, ImageFormat::WebP)
            .unwrap()
            .width(),
        16
    );
}
#[test]
fn manual_step_uses_displayed_index_after_skipped_or_inflight_frames() {
    let dir = TempDir::new();
    let w = worker(
        &dir,
        "gif",
        &gif(Some(image::codecs::gif::Repeat::Infinite)),
    );
    for index in [1, 2] {
        w.advance(None);
        assert_eq!(wait(&w).unwrap().unwrap().index, index);
    }
    // Both frames were late and not displayed. Step from displayed frame zero.
    w.advance(Some(0));
    let next = wait(&w).unwrap().unwrap();
    assert_eq!(next.index, 1);
    assert_eq!(&next.pixels.rgba[..4], &[0, 255, 0, 255]);
    assert!(next.pixels.accounted_bytes() <= 3 * next.pixels.rgba.len());
    w.advance(None); // May already be decoding when the manual command arrives.
    w.advance(Some(2));
    let next = wait(&w).unwrap().unwrap();
    assert_eq!(next.index, 0);
    assert_eq!(&next.pixels.rgba[..4], &[255, 0, 0, 255]);
}

#[test]
fn early_terminal_error_cannot_be_erased_by_first_advance() {
    let dir = TempDir::new();
    let w = worker(&dir, "gif", b"broken");
    let mut state = w.shared.state.lock().unwrap();
    while state.result.is_none() {
        let (new_state, timeout) = w
            .shared
            .wake
            .wait_timeout(state, Duration::from_secs(5))
            .unwrap();
        state = new_state;
        if timeout.timed_out() && state.result.is_none() {
            drop(state);
            panic!("terminal error was not delivered");
        }
    }
    assert!(state.closed);
    drop(state);
    w.advance(None);
    assert!(wait(&w).is_err());
}

#[test]
fn animation_reserve_rejects_large_canvas_before_frame_allocation() {
    let mut bytes = gif(None);
    bytes[6..8].copy_from_slice(&4096u16.to_le_bytes());
    bytes[8..10].copy_from_slice(&4096u16.to_le_bytes());
    assert!(matches!(
        Stream::open(Format::Standard(ImageFormat::Gif), bytes.into()),
        Err(LoadError::ResourceLimit(_))
    ));
}

#[test]
fn corrupt_animation_frames_error_without_panic() {
    let mut bytes = apng(0);
    let at = bytes.windows(4).position(|w| w == b"fdAT").unwrap();
    bytes[at + 8] ^= 0xff; // corrupt compressed second frame and its checksum
    let mut stream = Stream::open(Format::Standard(ImageFormat::Png), bytes.into())
        .unwrap()
        .unwrap();
    let first = stream.next().unwrap().unwrap();
    assert_eq!(&first.rgba[..4], &[255, 0, 0, 255]);
    assert!(stream.next().is_err());
}

#[test]
#[ignore = "Writes original animation fixtures for manual release smoke"]
fn write_smoke_fixtures() {
    let dir = std::env::temp_dir().join(format!("glance-animation-smoke-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(
        dir.join("animation.gif"),
        gif(Some(image::codecs::gif::Repeat::Infinite)),
    )
    .unwrap();
    std::fs::write(dir.join("animation.webp"), webp(0)).unwrap();
    std::fs::write(dir.join("animation.png"), apng(0)).unwrap();
    println!("Animation fixtures: {}", dir.display());
}
