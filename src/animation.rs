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
const FALLBACK_FRAME_DELAY: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rate {
    #[default]
    HonorFile,
    Exact,
    Fps30,
    Fps24,
    Fps15,
    Fps10,
    Off,
}

impl Rate {
    pub const ALL: [(Self, &'static str, &'static str); 7] = [
        (Self::HonorFile, "Honor file (max 50 fps)", "honor-file"),
        (Self::Exact, "Honor file exactly", "exact"),
        (Self::Fps30, "30 fps", "30-fps"),
        (Self::Fps24, "24 fps", "24-fps"),
        (Self::Fps15, "15 fps", "15-fps"),
        (Self::Fps10, "10 fps", "10-fps"),
        (Self::Off, "Off (first frame only)", "off"),
    ];

    pub fn key(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|entry| entry.0 == self)
            .expect("all rates")
            .2
    }

    pub fn parse(key: &str) -> Self {
        Self::ALL
            .iter()
            .find(|entry| entry.2 == key)
            .map_or(Self::default(), |entry| entry.0)
    }

    /// Off has no automatic interval; manual stepping still uses the default delay.
    pub fn interval(self, encoded: Duration) -> Duration {
        match self {
            Self::HonorFile | Self::Off if encoded <= Duration::from_millis(10) => {
                FALLBACK_FRAME_DELAY
            }
            Self::HonorFile | Self::Off => encoded.max(MIN_FRAME_DELAY),
            Self::Exact => encoded.max(Duration::from_millis(5)),
            Self::Fps30 => Duration::from_secs_f64(1.0 / 30.0),
            Self::Fps24 => Duration::from_secs_f64(1.0 / 24.0),
            Self::Fps15 => Duration::from_secs_f64(1.0 / 15.0),
            Self::Fps10 => Duration::from_millis(100),
        }
    }
}

fn encoded_delay(value: Option<(u32, u32)>) -> Duration {
    match value {
        None | Some((_, 0)) => FALLBACK_FRAME_DELAY,
        Some((n, d)) => Duration::from_secs_f64(f64::from(n) / f64::from(d) / 1000.0),
    }
}

#[cfg(test)]
pub fn delay(value: Option<(u32, u32)>) -> Duration {
    Rate::default().interval(encoded_delay(value))
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
        let timing = encoded_delay(Some(frame.delay().numer_denom_ms()));
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
/// Deadlines remain cumulative while on time, but never retain a backlog.
pub struct Clock {
    pub paused: bool,
    pub deadline: Duration,
    pub rate: Rate,
    remaining: Duration,
}
impl Clock {
    pub fn new(now: Duration, duration: Duration) -> Self {
        Self {
            paused: false,
            deadline: now.saturating_add(duration),
            rate: Rate::default(),
            remaining: duration,
        }
    }
    pub fn set_rate(&mut self, now: Duration, encoded: Duration, rate: Rate) {
        self.rate = rate;
        self.remaining = rate.interval(encoded);
        self.deadline = now.saturating_add(self.remaining);
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
    /// Account for a ready frame without dropping it. Decoding has already spent
    /// part (or all) of its interval: do not add another full interval on arrival.
    pub fn arrive(&mut self, now: Duration, duration: Duration) {
        self.advance(duration);
        self.deadline = self.deadline.max(now);
    }
    pub fn due(&self, now: Duration) -> bool {
        self.rate != Rate::Off && !self.paused && now >= self.deadline
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
    pub fn cancel(&self) {
        let mut state = self.shared.state.lock().expect("animation lock");
        if !state.closed {
            state.serial = state.serial.saturating_add(1);
            state.advance = None;
            state.result = None;
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
        if crate::folder::FileVersion::read(path)
            .ok()
            .is_none_or(|after| !version.same_content(&after))
        {
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
        // Decode dependents strictly in order, retaining only one lookahead frame.
        next = stream.next()?.map(Arc::new);
        let final_frame = next.is_none() && !may_repeat(loops, completed.saturating_add(1));
        if crate::folder::FileVersion::read(path)
            .ok()
            .is_none_or(|after| !version.same_content(&after))
        {
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
    fn rate_interval_table_and_exact_safety_floor() {
        let ms = Duration::from_millis;
        for (rate, _, _) in Rate::ALL {
            for (cs, default) in [
                (0, 100),
                (1, 100),
                (2, 20),
                (5, 50),
                (10, 100),
                (20, 200),
                (50, 500),
            ] {
                let expected = match rate {
                    Rate::HonorFile | Rate::Off => ms(default),
                    Rate::Exact => ms(cs * 10).max(ms(5)),
                    Rate::Fps30 => Duration::from_nanos(33_333_333),
                    Rate::Fps24 => Duration::from_nanos(41_666_667),
                    Rate::Fps15 => Duration::from_nanos(66_666_667),
                    Rate::Fps10 => ms(100),
                };
                assert_eq!(rate.interval(ms(cs * 10)), expected, "{rate:?}: {cs} cs");
            }
        }
        for value in [0, 1, 4, 5] {
            assert_eq!(Rate::Exact.interval(ms(value)), ms(5));
        }
        assert_eq!(
            Rate::Exact.interval(Duration::from_micros(12500)),
            Duration::from_micros(12500)
        );
    }

    #[test]
    fn live_rate_recomputes_deadline_and_preserves_pause() {
        let ms = Duration::from_millis;
        let mut clock = Clock::new(ms(0), ms(500));
        clock.set_rate(ms(40), ms(500), Rate::Fps10);
        assert_eq!(clock.deadline, ms(140));
        clock.advance(ms(100));
        assert_eq!(clock.deadline, ms(240)); // subsequent deadlines stay cumulative
        clock.toggle(ms(50));
        clock.set_rate(ms(60), ms(20), Rate::Exact);
        assert!(clock.paused);
        clock.toggle(ms(1000));
        assert_eq!(clock.deadline, ms(1020));
        clock.set_rate(ms(1010), ms(20), Rate::Off);
        assert!(!clock.due(ms(999999)));
        clock.step();
        clock.stepped(ms(50));
        assert!(clock.paused);
    }

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
    fn sustained_slow_delivery_rebases_without_an_extra_interval() {
        let ms = Duration::from_millis;
        let mut c = Clock::new(ms(0), ms(30));
        for arrival in [125, 220, 315, 410] {
            c.arrive(ms(arrival), ms(30));
            assert_eq!(c.deadline, ms(arrival));
            assert!(c.due(ms(arrival))); // next decode can start after admission
        }
        // Once decoding recovers, resume normal timing rather than catching up
        // through an unbounded backlog or adding a full interval after decoding.
        c.arrive(ms(415), ms(30));
        assert_eq!(c.deadline, ms(440));
        assert!(!c.due(ms(439)));
        assert!(c.due(ms(440)));
    }

    #[test]
    fn timely_arrivals_keep_requested_cadence_with_variable_delays() {
        let ms = Duration::from_millis;
        let mut c = Clock::new(ms(0), ms(100));
        c.arrive(ms(110), ms(50));
        assert_eq!(c.deadline, ms(150));
        c.arrive(ms(155), ms(200));
        assert_eq!(c.deadline, ms(350));
        c.arrive(ms(350), ms(30));
        assert_eq!(c.deadline, ms(380));
    }

    #[test]
    fn pause_and_manual_step_after_lateness_preserve_resume_timing() {
        let ms = Duration::from_millis;
        let mut c = Clock::new(ms(0), ms(30));
        c.arrive(ms(125), ms(30));
        c.toggle(ms(125));
        assert!(!c.due(ms(1000)));
        c.toggle(ms(1000));
        assert!(c.due(ms(1000)));
        c.arrive(ms(1005), ms(30));
        assert_eq!(c.deadline, ms(1030));
        c.step();
        c.stepped(ms(50));
        assert!(!c.due(ms(2000)));
        c.toggle(ms(2000));
        assert_eq!(c.deadline, ms(2050));
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
