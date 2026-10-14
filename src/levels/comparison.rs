//! Differential checks and opt-in, display-free CPU-stage timings.
use super::{Level, generate, generate_u8, generate16, reference};
use std::{hint::black_box, time::Instant};

fn assert_levels(actual: &[Level], expected: &[reference::Level]) {
    assert_eq!(actual.len(), expected.len());
    for (index, (a, b)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(a.size, b.size, "level {index} dimensions");
        assert_eq!(a.half, b.half, "level {index} format");
        // Do not print user-provided image bytes on failure.
        assert!(a.rgba == b.rgba, "level {index} payload differs");
    }
}

fn source(size: [usize; 2], pattern: usize) -> Vec<u8> {
    let mut state = 0x1234abcd_u32;
    (0..size[0] * size[1])
        .flat_map(|i| {
            let rgb: [u8; 3] = std::array::from_fn(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            });
            let alpha = match pattern {
                0 => 255,
                1 => 0,
                2 => (i % 256) as u8,
                3 => [0, 1, 127, 128, 254, 255][i % 6],
                _ => state as u8,
            };
            [rgb[0], rgb[1], rgb[2], alpha]
        })
        .collect()
}

#[test]
fn full_chains_match_original_for_even_odd_thin_and_alpha_patterns() {
    for size in [
        [1, 1],
        [1, 2],
        [2, 1],
        [1, 31],
        [31, 1],
        [2, 2],
        [2, 18],
        [18, 2],
        [3, 3],
        [4, 4],
        [6, 10],
        [7, 8],
        [8, 7],
        [15, 17],
        [32, 32],
        [64, 48],
        [127, 65],
        [256, 128],
    ] {
        for pattern in 0..5 {
            let input = source(size, pattern);
            assert_levels(
                &generate(size, &input).unwrap(),
                &reference::generate(size, &input).unwrap(),
            );
            // Exercise all sixteen bits, not only values exactly representable as U8.
            let input16: Vec<_> = input
                .iter()
                .enumerate()
                .map(|(i, &v)| {
                    if i % 4 == 3 && pattern < 2 {
                        u16::from(v) * 257
                    } else {
                        (u16::from(v) << 8) | (i as u16 & 255)
                    }
                })
                .collect();
            assert_levels(
                &generate16(size, &input16).unwrap(),
                &reference::generate16(size, &input16).unwrap(),
            );
        }
    }
}

#[test]
fn large_full_chains_match_independent_oracle_and_current_serial() {
    for size in [[1024, 1024], [1026, 1030], [1920, 1080]] {
        for pattern in [0, 3] {
            let input = source(size, pattern);
            let expected = reference::generate(size, &input).unwrap();
            assert_levels(&generate(size, &input).unwrap(), &expected);
            assert_levels(&generate_u8::<false>(size, &input).unwrap(), &expected);
        }
    }
}

#[test]
fn exhaustive_u8_tables_and_u16_input_domain_match_original() {
    for v in 0..=255 {
        assert_eq!(
            super::LINEAR[v].to_bits(),
            reference::linear(v as u8).to_bits()
        );
    }
    // Each RGB channel crosses every (encoded value, alpha) pair.
    let input: Vec<_> = (0..=255u8)
        .flat_map(|alpha| (0..=255u8).flat_map(move |v| [v, v.wrapping_add(85), 255 - v, alpha]))
        .collect();
    assert_levels(
        &generate([256, 256], &input).unwrap(),
        &reference::generate([256, 256], &input).unwrap(),
    );
    for alpha in [0, 1, 32768, 65535] {
        let input: Vec<_> = (0..=65535u16)
            .flat_map(|v| [v, v.wrapping_add(21845), 65535 - v, alpha])
            .collect();
        assert_levels(
            &generate16([256, 256], &input).unwrap(),
            &reference::generate16([256, 256], &input).unwrap(),
        );
    }
}

#[test]
fn invalid_dimensions_and_payload_errors_match_original() {
    for size in [
        [0, 0],
        [0, 4],
        [4, 0],
        [1, 1],
        [usize::MAX, 2],
        [16384, 16384],
    ] {
        for len in [0, 3, 5] {
            let input = vec![0; len];
            let actual = generate(size, &input).err().unwrap();
            let expected = reference::generate(size, &input).err().unwrap();
            assert!(matches!(actual, crate::decode::LoadError::ResourceLimit(_)));
            assert_eq!(actual.to_string(), expected.to_string());
            let input16 = vec![0; len];
            let actual = generate16(size, &input16).err().unwrap();
            let expected = reference::generate16(size, &input16).err().unwrap();
            assert!(matches!(actual, crate::decode::LoadError::ResourceLimit(_)));
            assert_eq!(actual.to_string(), expected.to_string());
        }
    }
}

fn timed<T>(run: impl FnOnce() -> T) -> f64 {
    let start = Instant::now();
    drop(black_box(run()));
    start.elapsed().as_secs_f64() * 1000.0
}

fn stats(label: &str, times: &mut [f64]) -> f64 {
    times.sort_by(f64::total_cmp);
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    let median = (times[(times.len() - 1) / 2] + times[times.len() / 2]) / 2.0;
    println!(
        "{label}: n={} mean={mean:.3} median={median:.3} range={:.3}..{:.3} ms",
        times.len(),
        times[0],
        times[times.len() - 1]
    );
    mean
}

fn benchmark(name: &str, size: [usize; 2], input: &[u8]) {
    assert_levels(
        &generate(size, input).unwrap(),
        &reference::generate(size, input).unwrap(),
    );
    let first_serial = timed(|| generate_u8::<false>(size, black_box(input)).unwrap());
    let first_parallel = timed(|| generate(size, black_box(input)).unwrap());
    println!(
        "{name}: first timed serial={first_serial:.3} parallel={first_parallel:.3} ms (tables warm, not cold OS caches)"
    );
    for _ in 0..2 {
        black_box(generate(size, black_box(input)).unwrap());
        black_box(generate_u8::<false>(size, black_box(input)).unwrap());
    }
    let mut original = Vec::new();
    let mut candidate = Vec::new();
    for i in 0..24 {
        let a = || generate_u8::<false>(black_box(size), black_box(input)).unwrap();
        let b = || generate(black_box(size), black_box(input)).unwrap();
        if i % 2 == 0 {
            original.push(timed(a));
            candidate.push(timed(b));
        } else {
            candidate.push(timed(b));
            original.push(timed(a));
        }
    }
    println!(
        "{name}: {}x{} U8 RGBA; complete level chain; allocation and drop included",
        size[0], size[1]
    );
    let a = stats("current optimized serial control", &mut original);
    let b = stats("candidate", &mut candidate);
    println!(
        "mean elapsed reduction={:.2}% ratio={:.3}x",
        (1.0 - b / a) * 100.0,
        a / b
    );
}

#[test]
#[ignore = "opt-in release benchmark; optionally reads GLANCE_LEVELS_BENCH_IMAGE locally"]
// Runtime-only guard: ordinary debug test builds must still compile this ignored test.
#[allow(clippy::assertions_on_constants)]
fn benchmark_levels() {
    assert!(!cfg!(debug_assertions), "run this benchmark with --release");
    // Run only this test in a fresh process for the first-touch sample. This is
    // table initialization, not a cold image decode or cold system-cache claim.
    let init = timed(|| {
        black_box(&*super::LINEAR);
        black_box(&*super::PREMULTIPLIED);
    });
    println!("first-touch U8 tables: {init:.3} ms (one sample; 66560 bytes static payload)");
    println!(
        "elapsed CPU-stage wall timings (not CPU counters or FPS); scoped helper creation/join, allocation/drop included; no timed decode/GPU"
    );
    println!(
        "available parallelism={:?}; helper cap=3, reserve one reported CPU, output threshold={} pixels",
        std::thread::available_parallelism(),
        super::parallel::MIN_OUTPUT_PIXELS
    );
    if let Some(path) = std::env::var_os("GLANCE_LEVELS_BENCH_IMAGE") {
        // Real normalization path; decode is outside timing and may use the LUTs.
        let pixels = crate::decode::load(std::path::Path::new(&path), 16384).unwrap();
        assert!(pixels.rgba16.is_none(), "benchmark fixture must be U8");
        let size = pixels.size;
        let input = pixels.rgba;
        drop(pixels.levels);
        benchmark(
            "local decoded first frame (tables already warm)",
            size,
            &input,
        );
    }
    for (name, size, pattern) in [
        ("generated small opaque", [256, 256], 0),
        ("generated below-threshold opaque", [510, 510], 0),
        ("generated threshold opaque", [512, 512], 0),
        ("generated square opaque", [1024, 1024], 0),
        ("generated opaque", [1920, 1080], 0),
        ("generated alpha ramp", [1920, 1080], 2),
        ("generated odd mixed alpha", [1919, 1079], 3),
        ("generated thin mixed alpha", [1, 2049], 4),
        ("generated large static opaque", [3840, 2160], 0),
    ] {
        benchmark(name, size, &source(size, pattern));
    }
}
