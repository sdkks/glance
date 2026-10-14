use super::*;
use std::sync::mpsc;

// Stop the real worker after publication but before successor normalization.
// Channels establish ordering; timeouts only diagnose a broken handshake.
fn hold_preparation(worker: &Worker, fail: bool) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    *worker.shared.before_prepare.lock().unwrap() = Some(Box::new(move || {
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        if fail {
            Err(LoadError::ResourceLimit(
                "injected preparation failure".into(),
            ))
        } else {
            Ok(())
        }
    }));
    (entered, release)
}

#[test]
fn publication_precedes_lookahead_preparation_without_an_extra_frame() {
    let dir = TempDir::new();
    for (ext, bytes) in [("gif", gif(None)), ("webp", webp(1)), ("png", apng(1))] {
        let w = worker(&dir, ext, &bytes);
        let (entered, release) = hold_preparation(&w, false);
        // APNG fixture has two frames: manually wrap to the first so a raw
        // successor exists at the seam, just as on automatic repeated plays.
        let target = (ext == "png").then_some(1);
        w.advance(target);
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let candidate = w
            .take()
            .expect("published before normalization")
            .unwrap()
            .unwrap();
        assert!(!candidate.final_frame);
        assert_eq!(candidate.index, if ext == "png" { 0 } else { 1 });
        assert!(w.shared.state.lock().unwrap().lookahead.is_none());
        assert_eq!(Arc::strong_count(&candidate.pixels), 1);
        // The sole worker successor is raw at this barrier, not an additional
        // normalized payload. Taking/requesting must not wait on pixel work.
        let weak = Arc::downgrade(&candidate.pixels);
        w.advance(None);
        assert!(w.take().is_none());
        release.send(()).unwrap();
        let next = wait(&w).unwrap().unwrap();
        assert!(next.final_frame);
        assert_eq!(next.index, candidate.index + 1);
        assert_eq!(weak.strong_count(), 1);
        drop(candidate);
        assert_eq!(weak.strong_count(), 0);
        w.advance(None);
        assert!(wait(&w).unwrap().is_none());
    }
}

#[test]
fn cancel_and_manual_target_during_preparation_keep_displayed_index_semantics() {
    let dir = TempDir::new();
    let w = worker(&dir, "gif", &gif(None));
    let (entered, release) = hold_preparation(&w, false);
    w.advance(None);
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    // Pause/rate changes cancel pending work; manual step is from the displayed
    // first frame, not the already advanced composition cursor.
    w.cancel();
    assert!(w.take().is_none());
    w.advance(Some(0));
    release.send(()).unwrap();
    let frame = wait(&w).unwrap().unwrap();
    assert_eq!(frame.index, 1);
    assert_eq!(&frame.pixels.rgba[..4], &[0, 255, 0, 255]);
    assert!(!frame.final_frame);
    w.advance(None);
    assert!(wait(&w).unwrap().unwrap().final_frame);
}

#[test]
fn preparation_failure_remains_terminal_before_or_after_taking_candidate() {
    let dir = TempDir::new();
    for take in [false, true] {
        let w = worker(&dir, "gif", &gif(None));
        let (entered, release) = hold_preparation(&w, true);
        w.advance(None);
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        if take {
            assert!(w.take().unwrap().unwrap().is_some());
        }
        release.send(()).unwrap();
        let mut state = w.shared.state.lock().unwrap();
        while !state.closed {
            let (updated, timeout) = w
                .shared
                .wake
                .wait_timeout(state, Duration::from_secs(5))
                .unwrap();
            state = updated;
            assert!(!timeout.timed_out(), "failure did not terminate worker");
        }
        drop(state);
        w.advance(None);
        w.cancel();
        assert!(matches!(wait(&w), Err(LoadError::ResourceLimit(message))
            if message == "injected preparation failure"));
    }
}

#[test]
fn dropping_worker_during_preparation_releases_candidate_and_exits() {
    let dir = TempDir::new();
    let path = dir.0.join("animation.gif");
    std::fs::write(&path, gif(None)).unwrap();
    let version = FileVersion::read(&path).unwrap();
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            advance: None,
            serial: 0,
            result: None,
            closed: false,
            lookahead: None,
        }),
        wake: Condvar::new(),
        context: eframe::egui::Context::default(),
        before_prepare: Mutex::new(None),
    });
    let w = Worker {
        shared: Arc::clone(&shared),
    };
    let (entered, release) = hold_preparation(&w, false);
    let (done_tx, done) = mpsc::channel();
    let child = Arc::clone(&shared);
    let thread = std::thread::spawn(move || done_tx.send(run(&child, &version)).unwrap());
    w.advance(None);
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    let candidate = w.take().unwrap().unwrap().unwrap();
    let weak = Arc::downgrade(&candidate.pixels);
    drop(candidate);
    drop(w);
    assert_eq!(weak.strong_count(), 0);
    release.send(()).unwrap();
    done.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
    thread.join().unwrap();
    let state = shared.state.lock().unwrap();
    assert!(state.closed);
    assert!(state.result.is_none());
    assert!(state.lookahead.is_none());
    assert_eq!(Arc::strong_count(&shared), 1);
}

#[test]
fn content_change_during_preparation_is_rejected_on_next_request() {
    let dir = TempDir::new();
    let w = worker(&dir, "gif", &gif(None));
    let (entered, release) = hold_preparation(&w, false);
    w.advance(None);
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(w.take().unwrap().unwrap().is_some());
    std::fs::write(dir.0.join("animation.gif"), b"changed").unwrap();
    w.advance(None);
    release.send(()).unwrap();
    assert!(matches!(wait(&w), Err(LoadError::Malformed(message))
        if message == "File changed during animation; reopen it"));
}
