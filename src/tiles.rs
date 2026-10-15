use crate::{
    decode::LoadError,
    levels::{Level, allocation},
};

pub const RESIDENT_BYTES: usize = 128 * 1024 * 1024;
pub const STAGING_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub level: usize,
    pub x: usize,
    pub y: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct Tile {
    pub origin: [usize; 2],
    pub size: [usize; 2],
}

pub fn core(device_limit: u32) -> Result<usize, LoadError> {
    if device_limit < 3 {
        return Err(LoadError::DisplayUnavailable(
            "Texture limit cannot hold filtering gutters".into(),
        ));
    }
    Ok((device_limit as usize - 2).min(1022))
}

pub fn tile(size: [usize; 2], key: Key, core: usize) -> Tile {
    let origin = [key.x * core, key.y * core];
    Tile {
        origin,
        size: [
            (size[0] - origin[0]).min(core),
            (size[1] - origin[1]).min(core),
        ],
    }
}

impl Tile {
    pub fn bytes_at_depth(self, bytes_per_pixel: usize) -> usize {
        (self.size[0] + 2) * (self.size[1] + 2) * bytes_per_pixel
    }
    pub fn domain(self, size: [usize; 2]) -> [f32; 4] {
        [
            self.origin[0] as f32 / size[0] as f32,
            self.origin[1] as f32 / size[1] as f32,
            (self.origin[0] + self.size[0]) as f32 / size[0] as f32,
            (self.origin[1] + self.size[1]) as f32 / size[1] as f32,
        ]
    }
}

pub fn gutter(level: &Level, tile: Tile) -> Result<Vec<u8>, LoadError> {
    let bpp = level.bytes_per_pixel();
    let mut data = allocation(tile.bytes_at_depth(bpp))?;
    let [ox, oy] = tile.origin;
    // Include real neighbors in the span; duplicate pixels only at image edges.
    let start = ox.saturating_sub(1);
    let end = (ox + tile.size[0] + 1).min(level.size[0]);
    let left = usize::from(ox == 0) * bpp;
    let span = (end - start) * bpp;
    let row_bytes = (tile.size[0] + 2) * bpp;
    for (y, row) in data.chunks_exact_mut(row_bytes).enumerate() {
        let sy = (oy + y).saturating_sub(1).min(level.size[1] - 1);
        let from = (sy * level.size[0] + start) * bpp;
        row[left..left + span].copy_from_slice(&level.rgba[from..from + span]);
        if left != 0 {
            row[..bpp].copy_from_slice(&level.rgba[from..from + bpp]);
        }
        if left + span != row_bytes {
            row[left + span..].copy_from_slice(&level.rgba[from + span - bpp..from + span]);
        }
    }
    Ok(data)
}

pub fn intersection(a: [f32; 4], b: [f32; 4]) -> Option<[f32; 4]> {
    let r = [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ];
    (r[0] < r[2] && r[1] < r[3]).then_some(r)
}

pub fn coverage(size: [usize; 2], level: usize, core: usize, visible: [f32; 4]) -> Vec<Key> {
    let Some(r) = intersection(visible, [0.0, 0.0, 1.0, 1.0]) else {
        return vec![];
    };
    let start = [
        (r[0] * size[0] as f32).floor() as usize / core,
        (r[1] * size[1] as f32).floor() as usize / core,
    ];
    let end = [
        (r[2] * size[0] as f32).ceil() as usize,
        (r[3] * size[1] as f32).ceil() as usize,
    ];
    let end = [
        end[0].min(size[0]).div_ceil(core),
        end[1].min(size[1]).div_ceil(core),
    ];
    (start[1]..end[1])
        .flat_map(|y| (start[0]..end[0]).map(move |x| Key { level, x, y }))
        .collect()
}

// At most eight neighbors, not a viewport-sized speculative ring.
pub fn adjacent(size: [usize; 2], core: usize, visible: &[Key]) -> Vec<Key> {
    let mut result = Vec::new();
    for key in visible {
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let Some(x) = key.x.checked_add_signed(dx) else {
                continue;
            };
            let Some(y) = key.y.checked_add_signed(dy) else {
                continue;
            };
            let next = Key {
                level: key.level,
                x,
                y,
            };
            if x < size[0].div_ceil(core)
                && y < size[1].div_ceil(core)
                && !visible.contains(&next)
                && !result.contains(&next)
            {
                result.push(next);
                if result.len() == 8 {
                    return result;
                }
            }
        }
    }
    result
}

// A completion witness can only be constructed for the entire required set.
pub struct CoverageReady {
    _complete: (),
}
pub fn ready(required: &[Key], contains: impl Fn(&Key) -> bool) -> Option<CoverageReady> {
    required
        .iter()
        .all(contains)
        .then_some(CoverageReady { _complete: () })
}

// The same recency/protection decision is used by GPU residency and synthetic tests.
pub fn eviction(keys: &[(Key, u64)], protected: &[Key]) -> Option<Key> {
    keys.iter()
        .filter(|(key, _)| !protected.contains(key))
        .min_by_key(|(_, age)| age)
        .map(|(key, _)| *key)
}

#[cfg(test)]
mod comparison;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synthetic_64_limit_covers_full_detail_all_corners() {
        let core = core(64).unwrap();
        let size = [10000, 6000];
        let all = coverage(size, 0, core, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(all.len(), size[0].div_ceil(core) * size[1].div_ceil(core));
        let mut area = 0;
        for key in &all {
            let t = tile(size, *key, core);
            assert!(t.size[0] + 2 <= 64 && t.size[1] + 2 <= 64);
            area += t.size[0] * t.size[1];
        }
        assert_eq!(area, size[0] * size[1]);
        for r in [
            [0.0, 0.0, 0.001, 0.001],
            [0.999, 0.0, 1.0, 0.001],
            [0.0, 0.999, 0.001, 1.0],
            [0.999, 0.999, 1.0, 1.0],
        ] {
            let visible = coverage(size, 0, core, r);
            assert!(!visible.is_empty());
            assert!(visible.iter().all(|k| all.contains(k)));
        }
    }
    #[test]
    fn gutters_copy_neighbors_not_independent_edge_colors() {
        let l = Level {
            size: [5, 3],
            rgba: (0..15).flat_map(|v| [v, v, v, 255]).collect(),
            half: false,
        };
        let left = tile(
            l.size,
            Key {
                level: 0,
                x: 0,
                y: 0,
            },
            3,
        );
        let right = tile(
            l.size,
            Key {
                level: 0,
                x: 1,
                y: 0,
            },
            3,
        );
        let a = gutter(&l, left).unwrap();
        let b = gutter(&l, right).unwrap();
        assert_eq!(a[4 * 4], 3); // right gutter is neighbor, not last interior texel 2.
        assert_eq!(b[0], 2); // left gutter.
        assert_eq!(a[0], 0); // exterior clamped.
        assert_eq!(b[(right.size[0] + 1) * 4], 4);
    }
    #[test]
    fn adjacency_is_bounded_and_eviction_protects_complete_visible_set() {
        let visible = coverage([1000, 1000], 0, 62, [0.3, 0.3, 0.7, 0.7]);
        let adjacent = adjacent([1000, 1000], 62, &visible);
        assert!(adjacent.len() <= 8);
        assert!(adjacent.iter().all(|k| !visible.contains(k)));
        let mut keys: Vec<_> = visible.iter().map(|k| (*k, 1)).collect();
        assert_eq!(eviction(&keys, &visible), None);
        keys.extend(adjacent.iter().enumerate().map(|(i, k)| (*k, i as u64 + 2)));
        assert_eq!(eviction(&keys, &visible), adjacent.first().copied());
    }
}
