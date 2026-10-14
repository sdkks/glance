//! Frozen from src/levels.rs at a58662e, through reduce16 (selection/tests omitted).
//! Keep formulas, accumulation order, allocation and validation independent of production.
use crate::decode::LoadError;

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
    let lut: [f32; 256] = std::array::from_fn(|v| linear(v as u8));
    let premultiplied: Vec<[u8; 256]> = (0..=255)
        .map(|alpha| std::array::from_fn(|v| encoded(lut[v] * alpha as f32 / 255.0)))
        .collect();
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
        levels.push(reduce(levels.last().unwrap(), &lut)?);
    }
    Ok(levels)
}

fn reduce(input: &Level, lut: &[f32; 256]) -> Result<Level, LoadError> {
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
