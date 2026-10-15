//! Byte-exact synthetic regression tests for the production gutter entry point.
use super::*;

// Frozen a975449 scalar gutter. No span calculation or candidate helpers.
fn oracle(level: &Level, tile: Tile) -> Result<Vec<u8>, LoadError> {
    let bpp = level.bytes_per_pixel();
    let mut data = allocation(tile.bytes_at_depth(bpp))?;
    let width = tile.size[0] + 2;
    for y in 0..tile.size[1] + 2 {
        let sy = (tile.origin[1] + y)
            .saturating_sub(1)
            .min(level.size[1] - 1);
        for x in 0..width {
            let sx = (tile.origin[0] + x)
                .saturating_sub(1)
                .min(level.size[0] - 1);
            let from = (sy * level.size[0] + sx) * bpp;
            let to = (y * width + x) * bpp;
            data[to..to + bpp].copy_from_slice(&level.rgba[from..from + bpp]);
        }
    }
    Ok(data)
}

fn synthetic(size: [usize; 2], half: bool, pattern: usize) -> Level {
    let bpp = if half { 8 } else { 4 };
    let rgba = (0..size[1])
        .flat_map(|y| {
            (0..size[0]).flat_map(move |x| {
                (0..bpp).map(move |c| {
                    if pattern == 0 {
                        (x * 37 + y * 71 + c * 109 + (x >> 8) * 17 + (y >> 8) * 23) as u8
                    } else {
                        let mut n = (y * size[0] * bpp + x * bpp + c) as u64;
                        n = n.wrapping_add(0x9e3779b97f4a7c15);
                        n = (n ^ (n >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                        n = (n ^ (n >> 27)).wrapping_mul(0x94d049bb133111eb);
                        (n ^ (n >> 31)) as u8
                    }
                })
            })
        })
        .collect();
    Level { size, rgba, half }
}

fn check(level: &Level, tile: Tile, pattern: usize) {
    let expected = oracle(level, tile).unwrap();
    let actual = gutter(level, tile).unwrap();
    assert_eq!(actual.len(), expected.len());
    if let Some(i) = actual.iter().zip(&expected).position(|(a, b)| a != b) {
        panic!(
            "byte mismatch: level={:?} half={} tile={tile:?} pattern={pattern} byte={i} actual={} expected={}",
            level.size, level.half, actual[i], expected[i]
        );
    }
}

#[test]
fn exact_bytes_all_small_valid_rectangles() {
    for half in [false, true] {
        for pattern in 0..2 {
            for w in 1..=9 {
                for h in 1..=9 {
                    let level = synthetic([w, h], half, pattern);
                    for oy in 0..h {
                        for ox in 0..w {
                            for th in 1..=h - oy {
                                for tw in 1..=w - ox {
                                    check(
                                        &level,
                                        Tile {
                                            origin: [ox, oy],
                                            size: [tw, th],
                                        },
                                        pattern,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn exact_bytes_small_grid_tiles_and_tiny_cores() {
    for half in [false, true] {
        for pattern in 0..2 {
            for w in 1..=9 {
                for h in 1..=9 {
                    let level = synthetic([w, h], half, pattern);
                    for core in 1..=5 {
                        for key in coverage(level.size, 0, core, [0.0, 0.0, 1.0, 1.0]) {
                            check(&level, tile(level.size, key, core), pattern);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn exact_bytes_production_boundaries_and_heavy_geometry() {
    for half in [false, true] {
        for pattern in 0..2 {
            for size in [
                [1021, 1023],
                [1022, 1022],
                [1023, 1021],
                [2044, 1022],
                [2045, 1023],
                [2045, 2045],
                [1, 2053],
                [2053, 1],
                [3, 2053],
                [2053, 3],
                [1280, 2052],
                [640, 1026],
            ] {
                let level = synthetic(size, half, pattern);
                for key in coverage(size, 0, 1022, [0.0, 0.0, 1.0, 1.0]) {
                    check(&level, tile(size, key, 1022), pattern);
                }
            }
        }
    }
}

#[test]
fn initialized_allocation_and_capacity_error_remain_fallible() {
    assert_eq!(allocation(17).unwrap(), [0; 17]);
    // Capacity overflow is rejected without attempting real OOM.
    assert!(matches!(
        allocation(usize::MAX),
        Err(LoadError::ResourceLimit(_))
    ));
}
