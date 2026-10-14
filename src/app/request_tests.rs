use super::*;
use std::time::Duration;

fn viewer() -> Viewer {
    let ctx = egui::Context::default();
    let mut viewer = Viewer::new(
        ctx.clone(),
        None,
        crate::render::Uploads::new(None),
        Default::default(),
        None,
    );
    viewer.display.status = Status::Ready;
    viewer.animation = Some(Playback {
        worker: crate::animation::Worker::idle_for_test(ctx),
        clock: crate::animation::Clock::new(Duration::ZERO, Duration::from_millis(40)),
        origin: std::time::Instant::now(),
        waiting: false,
        manual: false,
        final_frame: false,
        displayed_index: 4,
        candidate_index: 4,
        resync: false,
    });
    viewer
}

fn request(viewer: &mut Viewer, now_ms: u64) {
    viewer.request_animation(&viewer.context.clone(), Duration::from_millis(now_ms));
}

fn queued(viewer: &Viewer) -> (u64, Option<Option<usize>>) {
    viewer.animation.as_ref().unwrap().worker.request_for_test()
}

#[test]
fn end_of_ui_requests_after_admission_but_not_on_close_or_quit() {
    for (close, quit) in [(false, false), (true, false), (false, true)] {
        let mut viewer = viewer();
        viewer.animation.as_mut().unwrap().clock.deadline = Duration::ZERO;
        viewer.finish_animation_pass(
            &viewer.context.clone(),
            &crate::windows::Action {
                close,
                quit,
                ..Default::default()
            },
        );
        assert_eq!(queued(&viewer).0, u64::from(!close && !quit));
    }
}

#[test]
fn same_document_refresh_does_not_stall_requests() {
    let mut viewer = viewer();
    viewer.display.generation += 1;
    request(&mut viewer, 40);
    assert_eq!(queued(&viewer), (1, Some(None)));
}

#[test]
fn due_request_is_immediate_and_waiting_never_duplicates() {
    let mut viewer = viewer();
    request(&mut viewer, 40);
    assert_eq!(queued(&viewer), (1, Some(None)));
    assert!(viewer.animation.as_ref().unwrap().waiting);
    request(&mut viewer, 40);
    request(&mut viewer, 100);
    assert_eq!(queued(&viewer), (1, Some(None)));
}

#[test]
fn early_request_schedules_remaining_deadline_without_changing_cadence() {
    let mut viewer = viewer();
    let ctx = viewer.context.clone();
    // Drain egui's initial layout/font repaints without waiting on wall time.
    for _ in 0..3 {
        let mut warmup = ctx.run_ui(Default::default(), |_| {});
        warmup.textures_delta.clear();
    }
    let mut output = ctx.run_ui(
        egui::RawInput {
            predicted_dt: 0.0,
            ..Default::default()
        },
        |_| request(&mut viewer, 10),
    );
    output.textures_delta.clear();
    assert_eq!(queued(&viewer), (0, None));
    assert_eq!(
        viewer.animation.as_ref().unwrap().clock.deadline,
        Duration::from_millis(40)
    );
    assert_eq!(
        output.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
        Duration::from_millis(30)
    );
    request(&mut viewer, 40);
    assert_eq!(queued(&viewer), (1, Some(None)));
}

#[test]
fn pending_candidate_pause_off_manual_and_final_frame_do_not_advance() {
    for guard in 0..5 {
        let mut viewer = viewer();
        let playback = viewer.animation.as_mut().unwrap();
        match guard {
            0 => viewer.pending_animation = true,
            1 => playback.clock.toggle(Duration::from_millis(20)),
            2 => playback.clock.rate = crate::animation::Rate::Off,
            3 => playback.manual = true,
            4 => {
                playback.final_frame = true;
                playback.clock.step(); // successful final-frame admission
            }
            _ => unreachable!(),
        }
        request(&mut viewer, 100);
        assert_eq!(queued(&viewer), (0, None), "guard {guard}");
    }
}

#[test]
fn rate_cancel_resync_and_manual_request_keep_displayed_target() {
    let mut viewer = viewer();
    request(&mut viewer, 40);
    let playback = viewer.animation.as_mut().unwrap();
    playback.set_rate(Duration::from_millis(40), crate::animation::Rate::Fps10);
    let deadline = playback.clock.deadline;
    assert!(!playback.waiting);
    assert!(playback.resync);
    viewer.request_animation(&viewer.context.clone(), deadline - Duration::from_millis(1));
    assert_eq!(queued(&viewer), (2, None)); // cancellation, not a request
    viewer.request_animation(&viewer.context.clone(), deadline);
    assert_eq!(queued(&viewer), (3, Some(Some(4))));
    assert!(!viewer.animation.as_ref().unwrap().resync);
    let playback = viewer.animation.as_mut().unwrap();
    playback.clock.step();
    playback.manual = true;
    playback.waiting = true;
    playback.worker.advance(Some(playback.displayed_index));
    request(&mut viewer, 1000);
    assert_eq!(queued(&viewer), (4, Some(Some(4))));
    let playback = viewer.animation.as_mut().unwrap();
    playback.waiting = false;
    playback.manual = false;
    request(&mut viewer, 1000);
    assert_eq!(queued(&viewer), (4, Some(Some(4)))); // manual admission stays paused
}

#[test]
fn navigation_stale_admission_error_and_removed_worker_do_not_request() {
    for guard in 0..5 {
        let mut viewer = viewer();
        match guard {
            0 => viewer.display.status = Status::Loading,
            1 => viewer.selected = Some("other.gif".into()),
            2 => {
                // A stale candidate cannot clear waiting through commit_ready.
                viewer.animation.as_mut().unwrap().waiting = true;
                viewer.display.generation += 1;
            }
            3 => viewer.display.fail("decode failed".into()),
            4 => viewer.animation = None,
            _ => unreachable!(),
        }
        request(&mut viewer, 100);
        if viewer.animation.is_some() {
            assert_eq!(queued(&viewer), (0, None), "guard {guard}");
        }
    }
}

#[test]
fn discarded_pass_keeps_retired_paint_alive_and_defers_request_until_next_frame() {
    let mut viewer = viewer();
    let ctx = viewer.context.clone();
    let old = Arc::new(1_u8);
    let weak = Arc::downgrade(&old);
    let mut last_paint = egui::PaintCallback {
        rect: egui::Rect::EVERYTHING,
        callback: old,
    };
    let mut saw_second_pass = false;
    let mut output = ctx.run_ui(Default::default(), |ui| {
        if ctx.current_pass_index() == 0 {
            ui.painter().add(last_paint.clone());
            ctx.request_discard("exercise retained prior-pass callback");
        } else {
            saw_second_pass = true;
            last_paint.callback = Arc::new(2_u8);
            ui.painter().add(last_paint.clone());
            assert!(
                weak.upgrade().is_some(),
                "egui still owns the retired paint"
            );
            request(&mut viewer, 40);
            assert_eq!(queued(&viewer), (0, None));
        }
    });
    output.textures_delta.clear();
    assert!(saw_second_pass);
    assert_eq!(
        output.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
        Duration::ZERO
    );
    drop(output);
    let mut output = ctx.run_ui(Default::default(), |_| {
        assert!(weak.upgrade().is_none());
        request(&mut viewer, 40);
        assert_eq!(queued(&viewer), (1, Some(None)));
    });
    output.textures_delta.clear();
}

#[test]
fn admitted_frame_releases_old_display_and_egui_paint_before_request() {
    let mut viewer = viewer();
    let ctx = viewer.context.clone();
    let old = Arc::new(1_u8);
    let weak = Arc::downgrade(&old);
    let mut display = Display::new(Status::Ready);
    display.image = Some(old.clone());
    let mut last_paint = Some(egui::PaintCallback {
        rect: egui::Rect::EVERYTHING,
        callback: old,
    });
    // The backend owns the previous pass's output until painting is finished.
    let mut output = ctx.run_ui(Default::default(), |ui| {
        ui.painter().add(last_paint.clone().unwrap());
    });
    output.textures_delta.clear();
    drop(output);
    let mut output = ctx.run_ui(Default::default(), |ui| {
        let candidate = Arc::new(2_u8);
        assert!(display.finish_tiles(
            0,
            candidate.clone(),
            crate::tiles::ready(&[], |_| true),
            None,
            &mut View::default(),
        ));
        last_paint = Some(egui::PaintCallback {
            rect: egui::Rect::EVERYTHING,
            callback: candidate,
        });
        ui.painter().add(last_paint.clone().unwrap());
        assert!(weak.upgrade().is_none());
        request(&mut viewer, 40);
        assert_eq!(queued(&viewer), (1, Some(None)));
    });
    output.textures_delta.clear();
    drop(output);
}
