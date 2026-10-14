use crate::decode::LoadError;
use std::sync::LazyLock;

mod parallel;

// Invariant exact-formula tables, shared by all workers/windows (65 KiB payload).
static LINEAR: LazyLock<[f32; 256]> = LazyLock::new(|| std::array::from_fn(|v| linear(v as u8)));
static PREMULTIPLIED: LazyLock<[[u8; 256]; 256]> = LazyLock::new(|| {
    let lut = &*LINEAR;
    std::array::from_fn(|alpha| std::array::from_fn(|v| encoded(lut[v] * alpha as f32 / 255.0)))
});

#[cfg(test)]
mod comparison;
// Frozen pre-optimization implementation; no production helpers shared by the oracle.
#[cfg(test)]
#[allow(dead_code)]
mod reference;

// Rendering payload: sRGB-encoded *linear-premultiplied* RGB, linear alpha.
// The sRGB texture decodes RGB before filtering; the shader must not premultiply again.
// The straight RGBA8 source remains separate for lossless clipboard transforms.
pub struct Level {
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
    pub half: bool,
}

impl Level {
    pub fn bytes_per_pixel(&self) -> usize {
        if self.half { 8 } else { 4 }
    }
}

pub fn linear(value: u8) -> f32 {
    let v = f32::from(value) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear_unit(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

pub fn encoded_unit(value: f32) -> f32 {
    if value <= 0.0031308 {
        12.92 * value
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

fn encoded(value: f32) -> u8 {
    let v = if value <= 0.0031308 {
        12.92 * value
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (v * 255.0).round().clamp(0.0, 255.0) as u8
}

pub fn allocation(bytes: usize) -> Result<Vec<u8>, LoadError> {
    let mut data = Vec::new();
    data.try_reserve_exact(bytes)
        .map_err(|_| LoadError::ResourceLimit("Rendering allocation failed".into()))?;
    data.resize(bytes, 0);
    Ok(data)
}

pub fn generate(size: [usize; 2], straight: &[u8]) -> Result<Vec<Level>, LoadError> {
    generate_u8::<true>(size, straight)
}

// The false specialization is a test-only timing control: the current optimized
// serial algorithm, including shared tables and the exact same row kernel.
fn generate_u8<const PARALLEL: bool>(
    size: [usize; 2],
    straight: &[u8],
) -> Result<Vec<Level>, LoadError> {
    let bytes = size[0].checked_mul(size[1]).and_then(|n| n.checked_mul(4));
    if size.contains(&0)
        || bytes != Some(straight.len())
        || straight.len() as u64 > crate::decode::MAX_DECODED_BYTES
    {
        return Err(LoadError::ResourceLimit(
            "Invalid rendering source dimensions or payload".into(),
        ));
    }
    let mut rgba = allocation(straight.len())?;
    let lut = &*LINEAR;
    let premultiplied = &*PREMULTIPLIED;
    for (src, dst) in straight.chunks_exact(4).zip(rgba.chunks_exact_mut(4)) {
        for c in 0..3 {
            dst[c] = premultiplied[src[3] as usize][src[c] as usize];
        }
        dst[3] = src[3];
    }
    let mut levels = vec![Level {
        size,
        rgba,
        half: false,
    }];
    while levels.last().unwrap().size != [1, 1] {
        levels.push(reduce::<PARALLEL>(levels.last().unwrap(), lut)?);
    }
    Ok(levels)
}

fn reduce<const PARALLEL: bool>(input: &Level, lut: &[f32; 256]) -> Result<Level, LoadError> {
    let [w, h] = input.size;
    if w.is_multiple_of(2) && h.is_multiple_of(2) {
        let size = [w / 2, h / 2];
        let mut rgba = allocation(size[0] * size[1] * 4)?;
        if !PARALLEL || !parallel::reduce(input, lut, &mut rgba) {
            reduce_even_rows(&input.rgba, w * 4, lut, &mut rgba);
        }
        Ok(Level {
            size,
            rgba,
            half: false,
        })
    } else {
        reduce_generic(input, lut)
    }
}

// On measured release builds, keeping this boundary out of line avoids an
// odd-image slowdown. The generic partitioning and arithmetic stay unchanged.
#[inline(never)]
fn reduce_generic(input: &Level, lut: &[f32; 256]) -> Result<Level, LoadError> {
    let [w, h] = input.size;
    let size = [(w / 2).max(1), (h / 2).max(1)];
    let mut rgba = allocation(size[0] * size[1] * 4)?;
    for y in 0..size[1] {
        for x in 0..size[0] {
            // Floor-division boxes partition the complete parent, including odd remainders.
            let xs = x * w / size[0]..(x + 1) * w / size[0];
            let ys = y * h / size[1]..(y + 1) * h / size[1];
            let count = (xs.len() * ys.len()) as f32;
            let mut sum = [0.0; 4];
            for sy in ys {
                for sx in xs.clone() {
                    let p = &input.rgba[(sy * w + sx) * 4..][..4];
                    for c in 0..3 {
                        sum[c] += lut[p[c] as usize];
                    }
                    sum[3] += f32::from(p[3]) / 255.0;
                }
            }
            let dst = &mut rgba[(y * size[0] + x) * 4..][..4];
            for c in 0..3 {
                dst[c] = encoded(sum[c] / count);
            }
            dst[3] = (sum[3] / count * 255.0).round() as u8;
        }
    }
    Ok(Level {
        size,
        rgba,
        half: false,
    })
}

// Even two-dimensional parents have exact 2×2 boxes. Keep the generic path's
// top-left, top-right, bottom-left, bottom-right additions and alpha divisions.
fn reduce_even_rows(input: &[u8], row_bytes: usize, lut: &[f32; 256], rgba: &mut [u8]) {
    for (rows, output) in input
        .chunks_exact(row_bytes * 2)
        .zip(rgba.chunks_exact_mut(row_bytes / 2))
    {
        let (top, bottom) = rows.split_at(row_bytes);
        for ((a, b), dst) in top
            .chunks_exact(8)
            .zip(bottom.chunks_exact(8))
            .zip(output.chunks_exact_mut(4))
        {
            for c in 0..3 {
                let mut sum = 0.0;
                sum += lut[a[c] as usize];
                sum += lut[a[c + 4] as usize];
                sum += lut[b[c] as usize];
                sum += lut[b[c + 4] as usize];
                dst[c] = encoded(sum / 4.0);
            }
            let mut alpha = 0.0;
            alpha += f32::from(a[3]) / 255.0;
            alpha += f32::from(a[7]) / 255.0;
            alpha += f32::from(b[3]) / 255.0;
            alpha += f32::from(b[7]) / 255.0;
            dst[3] = (alpha / 4.0 * 255.0).round() as u8;
        }
    }
}

// U16 source stays straight, encoded sRGB. RGBA16F tiles and levels store
// linear-premultiplied RGB and linear alpha; no texture OETF/EOTF is needed.
pub fn generate16(size: [usize; 2], straight: &[u16]) -> Result<Vec<Level>, LoadError> {
    let samples = size[0].checked_mul(size[1]).and_then(|n| n.checked_mul(4));
    if size.contains(&0)
        || samples != Some(straight.len())
        || straight.len() as u64 * 2 > crate::decode::MAX_DECODED_BYTES
    {
        return Err(LoadError::ResourceLimit(
            "Invalid 16-bit rendering source".into(),
        ));
    }
    let mut rgba = allocation(straight.len() * 2)?;
    for (src, dst) in straight.chunks_exact(4).zip(rgba.chunks_exact_mut(8)) {
        let alpha = f32::from(src[3]) / 65535.0;
        for c in 0..4 {
            let value = if c == 3 {
                alpha
            } else {
                linear_unit(f32::from(src[c]) / 65535.0) * alpha
            };
            dst[c * 2..c * 2 + 2]
                .copy_from_slice(&half::f16::from_f32(value).to_bits().to_le_bytes());
        }
    }
    let mut levels = vec![Level {
        size,
        rgba,
        half: true,
    }];
    while levels.last().unwrap().size != [1, 1] {
        levels.push(reduce16(levels.last().unwrap())?);
    }
    Ok(levels)
}

fn reduce16(input: &Level) -> Result<Level, LoadError> {
    let [w, h] = input.size;
    let size = [(w / 2).max(1), (h / 2).max(1)];
    let mut rgba = allocation(size[0] * size[1] * 8)?;
    for y in 0..size[1] {
        for x in 0..size[0] {
            let xs = x * w / size[0]..(x + 1) * w / size[0];
            let ys = y * h / size[1]..(y + 1) * h / size[1];
            let count = (xs.len() * ys.len()) as f32;
            let mut sum = [0.0; 4];
            for sy in ys {
                for sx in xs.clone() {
                    let pixel = &input.rgba[(sy * w + sx) * 8..][..8];
                    for c in 0..4 {
                        sum[c] += half::f16::from_bits(u16::from_le_bytes([
                            pixel[c * 2],
                            pixel[c * 2 + 1],
                        ]))
                        .to_f32();
                    }
                }
            }
            let dst = &mut rgba[(y * size[0] + x) * 8..][..8];
            for c in 0..4 {
                dst[c * 2..c * 2 + 2]
                    .copy_from_slice(&half::f16::from_f32(sum[c] / count).to_bits().to_le_bytes());
            }
        }
    }
    Ok(Level {
        size,
        rgba,
        half: true,
    })
}

// Select using actual dimensions (not assumed powers of two for odd images).
// Each level transition blends continuously and keeps the residual reduction <= 2-ish.
pub fn select(levels: &[Level], physical_scale: f32) -> (usize, usize, f32) {
    if physical_scale >= 1.0 {
        return (0, 0, 0.0);
    }
    let base = levels[0].size;
    let footprint = |i: usize| {
        let size = levels[i].size;
        (base[0] as f32 / size[0] as f32).max(base[1] as f32 / size[1] as f32)
    };
    let desired = 1.0 / physical_scale.max(f32::MIN_POSITIVE);
    for i in 0..levels.len() - 1 {
        let a = footprint(i);
        let b = footprint(i + 1);
        if desired <= b && b > a {
            return (
                i,
                i + 1,
                ((desired / a).ln() / (b / a).ln()).clamp(0.0, 1.0),
            );
        }
    }
    let last = levels.len() - 1;
    (last, last, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn power_two_checker_is_box_average_without_shimmer() {
        let rgba: Vec<_> = (0..64)
            .flat_map(|i| {
                let v = if (i / 8 + i % 8) % 2 == 0 { 0 } else { 255 };
                [v, v, v, 255]
            })
            .collect();
        let levels = generate([8, 8], &rgba).unwrap();
        for level in &levels[1..] {
            for p in level.rgba.chunks_exact(4) {
                assert!((linear(p[0]) - 0.5).abs() < 0.004);
                assert_eq!(p[3], 255);
            }
        }
    }
    #[test]
    fn odd_floor_boxes_include_remainder_and_thin_axis() {
        let rgba: Vec<_> = (0..5)
            .flat_map(|i| [if i == 4 { 255 } else { 0 }, 0, 0, 255])
            .collect();
        let levels = generate([5, 1], &rgba).unwrap();
        assert_eq!(levels[1].size, [2, 1]);
        assert_eq!(levels[1].rgba[0], 0);
        assert!((linear(levels[1].rgba[4]) - 1.0 / 3.0).abs() < 0.004);
        let odd = generate([3, 3], &[255; 36]).unwrap();
        assert_eq!(odd[1].rgba, [255; 4]);
    }
    #[test]
    fn alpha_edge_is_premultiplied_once_and_roundtrips() {
        let levels = generate([2, 1], &[255, 0, 0, 255, 0, 255, 0, 0]).unwrap();
        assert_eq!(&levels[0].rgba[4..], &[0, 0, 0, 0]);
        let p = &levels[1].rgba;
        assert!((linear(p[0]) - 0.5).abs() < 0.004);
        assert_eq!(&p[1..], &[0, 0, 128]);
        for alpha in [32, 128, 255] {
            let result = generate([1, 1], &[128, 64, 32, alpha]).unwrap();
            let actual = linear(result[0].rgba[0]) / (f32::from(alpha) / 255.0);
            assert!((actual - linear(128)).abs() < 0.006);
        }
    }
    #[test]
    fn half_payload_premultiplies_once_zero_alpha_and_roundtrips() {
        for alpha in [0u16, 8192, 32768, 65535] {
            let source = [40000, 20000, 10000, alpha];
            let levels = generate16([1, 1], &source).unwrap();
            let p = &levels[0].rgba;
            let actual: [f32; 4] = std::array::from_fn(|c| {
                half::f16::from_bits(u16::from_le_bytes([p[c * 2], p[c * 2 + 1]])).to_f32()
            });
            if alpha == 0 {
                assert_eq!(actual, [0.0; 4]);
                continue;
            }
            for c in 0..3 {
                let straight = linear_unit(f32::from(source[c]) / 65535.0);
                assert!((actual[c] / actual[3] - straight).abs() < 0.001);
                assert!((straight * actual[3] - actual[c]).abs() < 0.001);
            }
        }
        let levels = generate16([2, 1], &[65535, 0, 0, 65535, 0, 65535, 0, 0]).unwrap();
        let p = &levels[1].rgba;
        assert_eq!(
            half::f16::from_bits(u16::from_le_bytes([p[0], p[1]])).to_f32(),
            0.5
        );
        assert_eq!(&p[2..6], &[0; 4]);
    }

    #[test]
    fn selection_blends_continuously_and_actual_size_uses_base() {
        let levels = generate([8, 8], &[255; 256]).unwrap();
        assert_eq!(select(&levels, 1.0), (0, 0, 0.0));
        assert_eq!(select(&levels, 2.0), (0, 0, 0.0));
        let (a, b, t) = select(&levels, 0.75);
        assert_eq!((a, b), (0, 1));
        assert!(t > 0.0 && t < 1.0);
        assert_eq!(select(&levels, 0.125), (2, 3, 1.0));
        assert_eq!(select(&levels, 0.01), (3, 3, 0.0));
    }
}
