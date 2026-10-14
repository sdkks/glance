//! Bounded, opportunistic parallelism for large even U8 reductions only.
use super::{Level, reduce_even_rows};
use std::{
    sync::{
        LazyLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

// One reduction owns the entire helper allowance across all windows/workers.
// Other callers never wait for it. Leave one reported CPU for other work even
// after counting the existing caller, and never add more than three helpers.
static BUSY: AtomicBool = AtomicBool::new(false);
static HELPERS: LazyLock<usize> = LazyLock::new(|| {
    helper_count(
        thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1),
    )
});
pub(super) const MIN_OUTPUT_PIXELS: usize = 256 * 256;

fn helper_count(cores: usize) -> usize {
    cores.saturating_sub(2).min(3)
}

struct Permit<'a>(&'a AtomicBool);
impl<'a> Permit<'a> {
    fn acquire(busy: &'a AtomicBool) -> Option<Self> {
        busy.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| Self(busy))
    }
}
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

// Explicit joins include thread-local destructors; scope's automatic wait only
// guarantees completion of the helper body. Drain every handle even on unwind.
struct Helpers<'scope>([Option<thread::ScopedJoinHandle<'scope, ()>>; 3]);
impl Drop for Helpers<'_> {
    fn drop(&mut self) {
        let mut panic = None;
        for handle in &mut self.0 {
            if let Some(handle) = handle.take()
                && let Err(payload) = handle.join()
                && panic.is_none()
            {
                panic = Some(payload);
            }
        }
        if !thread::panicking()
            && let Some(payload) = panic
        {
            std::panic::resume_unwind(payload);
        }
    }
}

pub(super) fn reduce(input: &Level, lut: &[f32; 256], output: &mut [u8]) -> bool {
    // Avoid querying platform parallelism on small levels.
    if output.len() / 4 < MIN_OUTPUT_PIXELS {
        return false;
    }
    reduce_with(input, lut, output, *HELPERS, &BUSY, || Ok(()))
}

// before_spawn is a zero-cost no-op in production and a deterministic failure
// injection seam in tests; the actual OS spawn always remains fallible.
fn reduce_with(
    input: &Level,
    lut: &[f32; 256],
    output: &mut [u8],
    helpers: usize,
    busy: &AtomicBool,
    mut before_spawn: impl FnMut() -> std::io::Result<()>,
) -> bool {
    let helpers = helpers.min(3).min((input.size[1] / 2).saturating_sub(1));
    if helpers == 0 || output.len() / 4 < MIN_OUTPUT_PIXELS {
        return false;
    }
    let Some(_permit) = Permit::acquire(busy) else {
        return false;
    };
    let row_bytes = input.size[0] * 4;
    let chunk_rows = (input.size[1] / 2).div_ceil(helpers + 1);
    // Permit lives outside scope: even on panic, every helper is joined before
    // another caller can claim capacity. A failed spawn drains prior helpers
    // before returning false so the caller can overwrite this same allocation.
    thread::scope(|scope| {
        let mut handles = Helpers([None, None, None]);
        let mut chunks = input
            .rgba
            .chunks(row_bytes * 2 * chunk_rows)
            .zip(output.chunks_mut(row_bytes / 2 * chunk_rows));
        let (caller_input, caller_output) = chunks.next().unwrap();
        for (index, (src, dst)) in chunks.enumerate() {
            let spawned = before_spawn().and_then(|()| {
                thread::Builder::new().spawn_scoped(scope, move || {
                    reduce_even_rows(src, row_bytes, lut, dst);
                })
            });
            match spawned {
                Ok(handle) => handles.0[index] = Some(handle),
                Err(_) => return false,
            }
        }
        reduce_even_rows(caller_input, row_bytes, lut, caller_output);
        true
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    fn fixture() -> Level {
        Level {
            size: [1024, 1024],
            rgba: (0..1024 * 1024 * 4)
                .map(|i| (i * 73 + i / 97) as u8)
                .collect(),
            half: false,
        }
    }

    #[test]
    fn capacity_threshold_and_unavailable_parallelism() {
        for (cores, expected) in [(0, 0), (1, 0), (2, 0), (3, 1), (4, 2), (5, 3), (128, 3)] {
            assert_eq!(helper_count(cores), expected);
        }
        let input = fixture();
        let busy = AtomicBool::new(false);
        let mut output = vec![0; 1024 * 1024];
        assert!(!reduce_with(
            &input,
            &super::super::LINEAR,
            &mut output,
            0,
            &busy,
            || panic!("must not spawn")
        ));
        assert!(!reduce_with(
            &input,
            &super::super::LINEAR,
            &mut output[..MIN_OUTPUT_PIXELS * 4 - 4],
            3,
            &busy,
            || panic!("must not spawn")
        ));
        let permit = Permit::acquire(&busy).unwrap();
        assert!(!reduce_with(
            &input,
            &super::super::LINEAR,
            &mut output,
            3,
            &busy,
            || panic!("must not spawn")
        ));
        drop(permit);
        assert!(Permit::acquire(&busy).is_some());
    }

    #[test]
    fn parallel_rows_and_partial_spawn_failure_match_serial() {
        let input = fixture();
        let lut = &super::super::LINEAR;
        let mut expected = vec![0; 1024 * 1024];
        reduce_even_rows(&input.rgba, 4096, lut, &mut expected);
        for fail_at in [None, Some(0), Some(1), Some(2)] {
            let busy = AtomicBool::new(false);
            let mut output = vec![97; expected.len()];
            let mut attempts = 0;
            let completed = reduce_with(&input, lut, &mut output, 3, &busy, || {
                assert!(busy.load(Ordering::Acquire));
                let fail = fail_at == Some(attempts);
                attempts += 1;
                if fail {
                    Err(std::io::Error::other("injected spawn failure"))
                } else {
                    Ok(())
                }
            });
            assert_eq!(completed, fail_at.is_none());
            assert_eq!(attempts, fail_at.map_or(3, |i| i + 1));
            if !completed {
                reduce_even_rows(&input.rgba, 4096, lut, &mut output);
            }
            assert!(output == expected);
            assert!(!busy.load(Ordering::Acquire));
        }
    }

    #[test]
    fn forced_helpers_match_independent_oracle_for_complete_large_chains() {
        // Local capacity makes this deterministic even on single-core CI or when
        // other tests hold the production permit. Exercise uneven row chunks too.
        for (size, helpers, opaque) in [
            ([1024, 1024], 1, true),
            ([1026, 1030], 2, false),
            ([2048, 2048], 3, false),
        ] {
            let straight: Vec<_> = (0..size[0] * size[1] * 4)
                .map(|i| {
                    if opaque && i % 4 == 3 {
                        255
                    } else {
                        (i * 73 + i / 97) as u8
                    }
                })
                .collect();
            let expected = super::super::reference::generate(size, &straight).unwrap();
            let mut levels = super::super::generate_u8::<false>(size, &straight).unwrap();
            levels.truncate(1);
            let busy = AtomicBool::new(false);
            let mut spawns = 0;
            while levels.last().unwrap().size != [1, 1] {
                let input = levels.last().unwrap();
                let [w, h] = input.size;
                let next = if w.is_multiple_of(2) && h.is_multiple_of(2) {
                    let mut rgba = vec![0; w * h];
                    if !reduce_with(
                        input,
                        &super::super::LINEAR,
                        &mut rgba,
                        helpers,
                        &busy,
                        || {
                            spawns += 1;
                            Ok(())
                        },
                    ) {
                        reduce_even_rows(&input.rgba, w * 4, &super::super::LINEAR, &mut rgba);
                    }
                    Level {
                        size: [w / 2, h / 2],
                        rgba,
                        half: false,
                    }
                } else {
                    super::super::reduce_generic(input, &super::super::LINEAR).unwrap()
                };
                levels.push(next);
            }
            assert!(
                spawns >= helpers,
                "must execute helpers, not silently fall back"
            );
            assert_eq!(levels.len(), expected.len());
            for (actual, expected) in levels.iter().zip(&expected) {
                assert_eq!(actual.size, expected.size);
                assert_eq!(actual.half, expected.half);
                assert!(actual.rgba == expected.rgba);
            }
            assert!(Permit::acquire(&busy).is_some());
        }
    }

    #[test]
    fn concurrent_callers_do_not_wait_or_reuse_capacity() {
        let busy = Arc::new(AtomicBool::new(false));
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        thread::scope(|scope| {
            let (worker_busy, worker_entered, worker_release) =
                (busy.clone(), entered.clone(), release.clone());
            scope.spawn(move || {
                let input = fixture();
                let mut output = vec![0; 1024 * 1024];
                let mut first = true;
                assert!(reduce_with(
                    &input,
                    &super::super::LINEAR,
                    &mut output,
                    3,
                    &worker_busy,
                    || {
                        if first {
                            first = false;
                            worker_entered.wait();
                            worker_release.wait();
                        }
                        Ok(())
                    }
                ));
            });
            entered.wait();
            let input = fixture();
            let mut output = vec![0; 1024 * 1024];
            assert!(!reduce_with(
                &input,
                &super::super::LINEAR,
                &mut output,
                3,
                &busy,
                || panic!("busy caller spawned")
            ));
            release.wait();
        });
        assert!(Permit::acquire(&busy).is_some());
    }

    #[test]
    fn unwind_after_partial_spawn_releases_actual_reduction_permit() {
        let busy = AtomicBool::new(false);
        let input = fixture();
        let mut output = vec![0; 1024 * 1024];
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut attempts = 0;
            reduce_with(&input, &super::super::LINEAR, &mut output, 3, &busy, || {
                attempts += 1;
                assert!(attempts < 2, "injected parent unwind after first spawn");
                Ok(())
            });
        }));
        assert!(result.is_err());
        assert!(Permit::acquire(&busy).is_some());
        let mut expected = vec![0; output.len()];
        reduce_even_rows(&input.rgba, 4096, &super::super::LINEAR, &mut expected);
        // The first helper's rows completed before unwind reached the caller.
        let chunk_bytes = output.len() / 4;
        assert!(output[chunk_bytes..2 * chunk_bytes] == expected[chunk_bytes..2 * chunk_bytes]);
    }

    #[test]
    fn explicit_join_retains_capacity_through_thread_local_destruction() {
        struct OnExit(Arc<Barrier>, Arc<Barrier>, Arc<AtomicBool>);
        impl Drop for OnExit {
            fn drop(&mut self) {
                self.0.wait();
                self.1.wait();
                self.2.store(true, Ordering::Release);
            }
        }
        thread_local! {
            static ON_EXIT: std::cell::RefCell<Option<OnExit>> = const { std::cell::RefCell::new(None) };
        }
        let busy = AtomicBool::new(false);
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let finished = Arc::new(AtomicBool::new(false));
        let held = thread::scope(|scope| {
            let worker_finished = finished.clone();
            let probe = OnExit(entered.clone(), release.clone(), finished.clone());
            let busy_ref = &busy;
            scope.spawn(move || {
                let _permit = Permit::acquire(busy_ref).unwrap();
                thread::scope(|scope| {
                    let _handles =
                        Helpers([
                            Some(scope.spawn(move || {
                                ON_EXIT.with(|slot| *slot.borrow_mut() = Some(probe))
                            })),
                            None,
                            None,
                        ]);
                });
                assert!(worker_finished.load(Ordering::Acquire));
            });
            entered.wait();
            let held = Permit::acquire(&busy).is_none();
            // Release before assertions so a failure cannot strand a helper.
            release.wait();
            held
        });
        assert!(held);
        assert!(finished.load(Ordering::Acquire));
        assert!(Permit::acquire(&busy).is_some());
    }

    #[test]
    fn permit_survives_scope_until_helpers_join_including_unwind() {
        let busy = AtomicBool::new(false);
        let result = std::panic::catch_unwind(|| {
            let _permit = Permit::acquire(&busy).unwrap();
            thread::scope(|scope| {
                let _handles = Helpers([
                    Some(scope.spawn(|| {
                        assert!(Permit::acquire(&busy).is_none());
                        panic!("injected helper panic");
                    })),
                    None,
                    None,
                ]);
            });
        });
        assert!(result.is_err());
        assert!(Permit::acquire(&busy).is_some());
    }
}
