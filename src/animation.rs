//! Pull-driven composed frames. The decoder stays on its worker; no frame collection.
use crate::decode::{LoadError, MAX_DECODED_BYTES, MAX_DIMENSION, Pixels};
use image::{
    AnimationDecoder, ImageDecoder, ImageFormat, Limits,
    metadata::{LoopCount, Orientation},
};
use std::{
    io::Cursor,
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

/// Default 50 fps ceiling; fractional delays above this floor remain intact.
pub const MIN_FRAME_DELAY: Duration = Duration::from_millis(20);
const SHORT_DELAY_THRESHOLD_MS: f64 = 10.0;
const FALLBACK_FRAME_DELAY: Duration = Duration::from_millis(100);

pub fn delay(value: Option<(u32, u32)>) -> Duration {
    match value {
        None | Some((0, _)) | Some((_, 0)) => FALLBACK_FRAME_DELAY,
        Some((n, d)) => {
            let ms = f64::from(n) / f64::from(d);
            // Browser-compatible: 0–1 centisecond delays use 100 ms, not a fast floor.
            if ms <= SHORT_DELAY_THRESHOLD_MS {
                FALLBACK_FRAME_DELAY
            } else {
                Duration::from_secs_f64(ms / 1000.0).max(MIN_FRAME_DELAY)
            }
        }
    }
}

// image maps an absent GIF extension to Infinite. Walk container blocks (never
// compressed data byte patterns) to preserve the distinction. Netscape's finite
// value counts repeats after the initial play; APNG/WebP count total plays.
fn gif_loops(bytes: &[u8]) -> Result<LoopCount, LoadError> {
    let malformed = || LoadError::Malformed("GIF: invalid or truncated block metadata".into());
    if bytes.len() < 13 || !matches!(&bytes[..6], b"GIF87a" | b"GIF89a") {
        return Err(malformed());
    }
    let mut position = 13;
    let table_size = |packed: u8| {
        if packed & 0x80 == 0 {
            0
        } else {
            3usize * (1 << ((packed & 7) + 1))
        }
    };
    position += table_size(bytes[10]);
    let mut loops = LoopCount::Finite(std::num::NonZeroU32::new(1).expect("one"));
    loop {
        let tag = *bytes.get(position).ok_or_else(malformed)?;
        position += 1;
        let mut application = false;
        match tag {
            0x3b => return Ok(loops),
            0x2c => {
                let descriptor = bytes.get(position..position + 9).ok_or_else(malformed)?;
                position += 9 + table_size(descriptor[8]);
                bytes.get(position).ok_or_else(malformed)?; // LZW code size
                position += 1;
            }
            0x21 => {
                let label = *bytes.get(position).ok_or_else(malformed)?;
                position += 1;
                if label == 0xff {
                    let length = usize::from(*bytes.get(position).ok_or_else(malformed)?);
                    position += 1;
                    let identifier = bytes
                        .get(position..position + length)
                        .ok_or_else(malformed)?;
                    application = identifier == b"NETSCAPE2.0" || identifier == b"ANIMEXTS1.0";
                    position += length;
                }
            }
            _ => return Err(malformed()),
        }
        loop {
            let length = usize::from(*bytes.get(position).ok_or_else(malformed)?);
            position += 1;
            if length == 0 {
                break;
            }
            let block = bytes
                .get(position..position + length)
                .ok_or_else(malformed)?;
            if application && length == 3 && block[0] == 1 {
                let repeats = u16::from_le_bytes([block[1], block[2]]);
                loops = if repeats == 0 {
                    LoopCount::Infinite
                } else {
                    LoopCount::Finite(
                        std::num::NonZeroU32::new(u32::from(repeats) + 1)
                            .expect("positive repeats"),
                    )
                };
            }
            position += length;
        }
    }
}

type ColorMetadata = (Option<Vec<u8>>, Orientation, Option<String>);

pub struct Stream {
    frames: image::Frames<'static>,
    pub loops: LoopCount,
    icc: Option<Vec<u8>>,
    orientation: Orientation,
    warning: Option<String>,
}

impl Stream {
    pub fn open(
        format: crate::formats::Format,
        bytes: Arc<[u8]>,
    ) -> Result<Option<Self>, LoadError> {
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_DIMENSION);
        limits.max_image_height = Some(MAX_DIMENSION);
        limits.max_alloc = Some(MAX_DECODED_BYTES / 4);
        fn metadata(
            decoder: &mut impl ImageDecoder,
            limits: Limits,
        ) -> Result<ColorMetadata, LoadError> {
            decoder.set_limits(limits)?;
            let (w, h) = decoder.dimensions();
            // Reserve three normalized frames (source + levels <= 3x RGBA),
            // plus three RGBA compositor/reference buffers inside the codec.
            if w == 0
                || h == 0
                || w > MAX_DIMENSION
                || h > MAX_DIMENSION
                || u64::from(w) * u64::from(h) * 4 > MAX_DECODED_BYTES / 12
            {
                return Err(LoadError::ResourceLimit(
                    "Animation frames and compositor reserve exceed 512 MiB".into(),
                ));
            }
            let (icc, warning) = match decoder.icc_profile() {
                Ok(p) => (p, None),
                Err(_) => (None, Some(crate::color::FALLBACK.to_owned())),
            };
            Ok((icc, decoder.orientation()?, warning))
        }
        let gif_loops = if matches!(format, crate::formats::Format::Standard(ImageFormat::Gif)) {
            Some(gif_loops(&bytes)?)
        } else {
            None
        };
        let cursor = Cursor::new(bytes);
        let (frames, loops, icc, orientation, warning) = match format {
            crate::formats::Format::Standard(ImageFormat::Gif) => {
                let mut d = image::codecs::gif::GifDecoder::new(cursor)?;
                let (icc, orientation, warning) = metadata(&mut d, limits)?;
                let loops = gif_loops.expect("GIF metadata checked");
                (d.into_frames(), loops, icc, orientation, warning)
            }
            crate::formats::Format::Standard(ImageFormat::Png) => {
                let mut d = image::codecs::png::PngDecoder::new(cursor)?;
                if !d.is_apng()? {
                    return Ok(None);
                }
                let (icc, orientation, warning) = metadata(&mut d, limits)?;
                let d = d.apng()?;
                let loops = d.loop_count();
                (d.into_frames(), loops, icc, orientation, warning)
            }
            crate::formats::Format::Standard(ImageFormat::WebP) => {
                let mut d = image::codecs::webp::WebPDecoder::new(cursor)?;
                if !d.has_animation() {
                    return Ok(None);
                }
                let (icc, orientation, warning) = metadata(&mut d, limits)?;
                let loops = d.loop_count();
                (d.into_frames(), loops, icc, orientation, warning)
            }
            _ => return Ok(None),
        };
        Ok(Some(Self {
            frames,
            loops,
            icc,
            orientation,
            warning,
        }))
    }

    pub fn next(&mut self) -> Result<Option<Pixels>, LoadError> {
        let Some(frame) = self.frames.next() else {
            return Ok(None);
        };
        let frame = frame?;
        let timing = delay(Some(frame.delay().numer_denom_ms()));
        let mut image = image::DynamicImage::ImageRgba8(frame.into_buffer());
        image.apply_orientation(self.orientation);
        let size = [image.width() as usize, image.height() as usize];
        let mut rgba = image.into_rgba8().into_raw();
        let warning = crate::color::normalize8(&mut rgba, self.icc.as_deref(), false)
            .or_else(|| self.warning.clone());
        let levels = crate::levels::generate(size, &rgba)?;
        Ok(Some(Pixels {
            size,
            rgba,
            levels,
            warning,
            animation_delay: Some(timing),
            ..Default::default()
        }))
    }
}

/// Clock values are elapsed monotonic durations, injected directly in tests.
/// Deadlines advance from their old value, never from a late presentation time.
pub struct Clock {
    pub paused: bool,
    pub deadline: Duration,
    remaining: Duration,
}
impl Clock {
    pub fn new(now: Duration, duration: Duration) -> Self {
        Self {
            paused: false,
            deadline: now.saturating_add(duration),
            remaining: duration,
        }
    }
    pub fn toggle(&mut self, now: Duration) {
        if self.paused {
            self.deadline = now.saturating_add(self.remaining);
        } else {
            self.remaining = self.deadline.saturating_sub(now);
        }
        self.paused = !self.paused;
    }
    pub fn advance(&mut self, duration: Duration) {
        self.deadline = self.deadline.saturating_add(duration);
    }
    pub fn arrive(&mut self, now: Duration, duration: Duration, final_frame: bool) -> bool {
        self.advance(duration);
        !final_frame && self.due(now)
    }
    pub fn due(&self, now: Duration) -> bool {
        !self.paused && now >= self.deadline
    }
    pub fn step(&mut self) {
        self.paused = true;
    }
    pub fn stepped(&mut self, duration: Duration) {
        self.remaining = duration;
    }
}

pub fn may_repeat(loops: LoopCount, completed: u32) -> bool {
    match loops {
        LoopCount::Infinite => true,
        LoopCount::Finite(n) => completed < n.get(),
    }
}

pub struct Frame {
    pub pixels: Arc<Pixels>,
    pub final_frame: bool,
    pub index: usize,
}
struct State {
    advance: Option<(u64, Option<usize>)>,
    serial: u64,
    result: Option<Result<Option<Frame>, LoadError>>,
    closed: bool,
    #[cfg(test)]
    lookahead: Option<std::sync::Weak<Pixels>>,
}
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    context: eframe::egui::Context,
}
pub struct Worker {
    shared: Arc<Shared>,
}

impl Worker {
    pub fn new(
        version: crate::folder::FileVersion,
        context: eframe::egui::Context,
    ) -> Result<Self, LoadError> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                advance: None,
                serial: 0,
                result: None,
                closed: false,
                #[cfg(test)]
                lookahead: None,
            }),
            wake: Condvar::new(),
            context,
        });
        let child = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("glance-animation".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(&child, &version)
                }));
                let error = match result {
                    Ok(Ok(())) => return,
                    Ok(Err(error)) => error,
                    Err(_) => {
                        LoadError::Malformed("Animation decoder panicked on malformed input".into())
                    }
                };
                let mut state = child.state.lock().expect("animation lock");
                if !state.closed {
                    state.closed = true; // Terminal failure: later requests must not erase this result.
                    state.result = Some(Err(error));
                    child.wake.notify_all();
                    child.context.request_repaint();
                }
            })?;
        Ok(Self { shared })
    }
    // A manual target is the displayed frame's index, not the decoder cursor,
    // which may have moved ahead while catching up. Replace stale work/results.
    pub fn advance(&self, target: Option<usize>) {
        let mut state = self.shared.state.lock().expect("animation lock");
        if !state.closed {
            state.serial = state.serial.saturating_add(1);
            state.advance = Some((state.serial, target));
            state.result = None;
            self.shared.wake.notify_one();
        }
    }
    pub fn take(&self) -> Option<Result<Option<Frame>, LoadError>> {
        self.shared
            .state
            .lock()
            .expect("animation lock")
            .result
            .take()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().expect("animation lock");
        state.closed = true;
        state.result = None;
        self.shared.wake.notify_one();
    }
}

fn run(shared: &Shared, version: &crate::folder::FileVersion) -> Result<(), LoadError> {
    let path = &version.path;
    let format = crate::formats::format(path).ok_or(LoadError::Unsupported)?;
    let bytes: Arc<[u8]> = crate::decode::read_encoded(path)?.into();
    let reopen = || {
        Stream::open(format, Arc::clone(&bytes))?
            .ok_or_else(|| LoadError::Malformed("Animation changed to a static image".into()))
    };
    let mut stream = reopen()?;
    let loops = stream.loops;
    // Foreground admission already displayed the first composed frame.
    stream
        .next()?
        .ok_or_else(|| LoadError::Malformed("Empty animation".into()))?;
    let mut next = stream.next()?.map(Arc::new);
    let mut completed = 0u32;
    let mut index = 1usize;
    'requests: loop {
        let (serial, target) = {
            let mut state = shared.state.lock().expect("animation lock");
            #[cfg(test)]
            {
                state.lookahead = next.as_ref().map(Arc::downgrade);
            }
            while state.advance.is_none() && !state.closed {
                state = shared.wake.wait(state).expect("animation lock");
            }
            if state.closed {
                return Ok(());
            }
            state.advance.take().expect("advance checked")
        };
        if crate::folder::FileVersion::read(path).ok().as_ref() != Some(version) {
            return Err(LoadError::Malformed(
                "File changed during animation; reopen it".into(),
            ));
        }
        if let Some(target) = target {
            // Replay composition from the beginning for exact manual inspection.
            // Drop lookahead before reopening; never retain a loop's frame history.
            drop(next.take());
            drop(stream);
            stream = reopen()?;
            for _ in 0..=target {
                {
                    let state = shared.state.lock().expect("animation lock");
                    if state.closed {
                        return Ok(());
                    }
                    if state.serial != serial {
                        continue 'requests;
                    }
                }
                stream
                    .next()?
                    .ok_or_else(|| LoadError::Malformed("Animation frame disappeared".into()))?;
            }
            next = stream.next()?.map(Arc::new);
            index = target.saturating_add(1);
        }
        if next.is_none() {
            completed = completed.saturating_add(1);
            if target.is_none() && !may_repeat(loops, completed) {
                let mut state = shared.state.lock().expect("animation lock");
                if !state.closed && state.serial == serial {
                    state.result = Some(Ok(None));
                    shared.wake.notify_all();
                    shared.context.request_repaint();
                }
                continue;
            }
            drop(stream);
            stream = reopen()?;
            index = 0;
            next = stream.next()?.map(Arc::new);
        }
        let pixels = next
            .take()
            .ok_or_else(|| LoadError::Malformed("Empty animation".into()))?;
        // Decode dependents strictly in order, even when presentation skips them.
        next = stream.next()?.map(Arc::new);
        let final_frame = next.is_none() && !may_repeat(loops, completed.saturating_add(1));
        if crate::folder::FileVersion::read(path).ok().as_ref() != Some(version) {
            return Err(LoadError::Malformed(
                "File changed during animation; reopen it".into(),
            ));
        }
        let mut state = shared.state.lock().expect("animation lock");
        if state.closed {
            return Ok(());
        }
        if state.serial != serial {
            continue;
        }
        #[cfg(test)]
        {
            state.lookahead = next.as_ref().map(Arc::downgrade);
        }
        state.result = Some(Ok(Some(Frame {
            pixels,
            final_frame,
            index,
        })));
        index = index.saturating_add(1);
        shared.wake.notify_all();
        shared.context.request_repaint();
    }
}

pub fn first(format: crate::formats::Format, bytes: &[u8]) -> Result<Option<Pixels>, LoadError> {
    match Stream::open(format, Arc::from(bytes))? {
        Some(mut stream) => stream
            .next()?
            .map(Some)
            .ok_or_else(|| LoadError::Malformed("Empty animation".into())),
        None => Ok(None),
    }
}

#[cfg(test)]
mod fixtures;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalization_matrix() {
        for (input, ms) in [
            (None, 100),
            (Some((0, 1)), 100),
            (Some((1, 0)), 100),
            (Some((1, 100)), 100),
            (Some((9, 1)), 100),
            // GIF centiseconds: 0, 1, 2, 5, 10, 20 → 100, 100, 20, 50, 100, 200 ms.
            (Some((10, 1)), 100),
            (Some((20, 1)), 20),
            (Some((50, 1)), 50),
            (Some((100, 1)), 100),
            (Some((200, 1)), 200),
            (Some((125, 1)), 125),
            (Some((10000, 1000)), 100),
            (Some((10001, 1000)), 20),
            (Some((25, 2)), 20),
            (Some((19999, 1000)), 20),
        ] {
            assert_eq!(delay(input), Duration::from_millis(ms), "{input:?}");
        }
        assert_eq!(delay(Some((41, 2))), Duration::from_micros(20500));
    }
    #[test]
    fn finite_infinite_once() {
        for n in [1, 2, 10] {
            let loops = LoopCount::Finite(std::num::NonZeroU32::new(n).unwrap());
            for completed in 0..20 {
                assert_eq!(may_repeat(loops, completed), completed < n);
            }
        }
        assert!(may_repeat(LoopCount::Infinite, u32::MAX));
    }
    #[test]
    fn late_frames_skip_but_final_frame_is_presented_without_drift() {
        let ms = Duration::from_millis;
        let mut c = Clock::new(ms(0), ms(30));
        assert!(c.arrive(ms(125), ms(30), false));
        assert_eq!(c.deadline, ms(60));
        assert!(c.arrive(ms(125), ms(30), false));
        assert_eq!(c.deadline, ms(90));
        assert!(!c.arrive(ms(125), ms(30), true));
        assert_eq!(c.deadline, ms(120));
        assert!(!c.arrive(ms(125), ms(30), false));
        assert_eq!(c.deadline, ms(150));
    }

    #[test]
    fn pause_step_reset_and_catchup_do_not_drift() {
        let ms = Duration::from_millis;
        let mut c = Clock::new(ms(0), ms(100));
        c.toggle(ms(35));
        assert!(!c.due(ms(1000)));
        c.toggle(ms(1000));
        assert_eq!(c.deadline, ms(1065));
        c.advance(ms(100));
        c.advance(ms(100));
        assert_eq!(c.deadline, ms(1265));
        assert!(c.due(ms(12500)));
        c.step();
        c.stepped(ms(50));
        c.toggle(ms(20000));
        assert_eq!(c.deadline, ms(20050));
        let new = Clock::new(ms(20000), ms(100));
        assert!(!new.paused);
        assert_eq!(new.deadline, ms(20100));
    }
}
