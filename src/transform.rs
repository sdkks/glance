use crate::decode::{LoadError, MAX_DECODED_BYTES, Pixels};

#[derive(Clone, Copy, Debug)]
pub enum Operation {
    Clockwise,
    Counterclockwise,
    Mirror,
    Flip,
}

// Signed permutation from oriented source axes to displayed axes. Translation
// follows from the signs; only the eight pixel-exact square symmetries exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transform([[i8; 2]; 2]);

impl Default for Transform {
    fn default() -> Self {
        Self([[1, 0], [0, 1]])
    }
}

impl Transform {
    pub fn apply(&mut self, operation: Operation) {
        let op = match operation {
            Operation::Clockwise => [[0, -1], [1, 0]],
            Operation::Counterclockwise => [[0, 1], [-1, 0]],
            Operation::Mirror => [[-1, 0], [0, 1]],
            Operation::Flip => [[1, 0], [0, -1]],
        };
        let old = self.0;
        self.0 = op.map(|coefficients| {
            std::array::from_fn(|column| {
                coefficients[0] * old[0][column] + coefficients[1] * old[1][column]
            })
        });
    }

    pub fn size(self, source: [usize; 2]) -> [usize; 2] {
        if self.0[0][0] == 0 {
            [source[1], source[0]]
        } else {
            source
        }
    }

    // Rows of the inverse UV mapping, including translation and uniform padding.
    // The renderer and CPU copy both use this same inverse signed permutation.
    pub fn uv_rows(self) -> [[f32; 4]; 2] {
        std::array::from_fn(|axis| {
            let x = self.0[0][axis];
            let y = self.0[1][axis];
            [
                f32::from(x),
                f32::from(y),
                if x + y < 0 { 1.0 } else { 0.0 },
                0.0,
            ]
        })
    }

    fn source_pixel(self, output: [usize; 2], source: [usize; 2]) -> [usize; 2] {
        std::array::from_fn(|axis| {
            let x = self.0[0][axis];
            let y = self.0[1][axis];
            let coordinate = output[usize::from(x == 0)];
            if x + y < 0 {
                source[axis] - 1 - coordinate
            } else {
                coordinate
            }
        })
    }

    pub fn materialize(self, pixels: &Pixels) -> Result<Pixels, LoadError> {
        let [width, height] = pixels.size;
        let bytes = width
            .checked_mul(height)
            .and_then(|n| n.checked_mul(4))
            .filter(|n| *n as u64 <= MAX_DECODED_BYTES)
            .ok_or_else(|| LoadError::ResourceLimit("Copy Image RGBA8 size".into()))?;
        if width == 0 || height == 0 || bytes != pixels.rgba.len() {
            return Err(LoadError::ResourceLimit(
                "Copy Image invalid pixel payload".into(),
            ));
        }
        let size = self.size(pixels.size);
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(bytes)
            .map_err(|_| LoadError::ResourceLimit("Copy Image allocation failed".into()))?;
        rgba.resize(bytes, 0);
        for y in 0..size[1] {
            for x in 0..size[0] {
                let [sx, sy] = self.source_pixel([x, y], pixels.size);
                let from = (sy * width + sx) * 4;
                let to = (y * size[0] + x) * 4;
                rgba[to..to + 4].copy_from_slice(&pixels.rgba[from..from + 4]);
            }
        }
        Ok(Pixels {
            size,
            rgba,
            levels: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Pixels {
        Pixels {
            size: [3, 2],
            rgba: (0..6).flat_map(|n| [n, n + 10, n + 20, n * 40]).collect(),
            levels: vec![],
        }
    }
    fn labels(pixels: &Pixels) -> Vec<u8> {
        pixels.rgba.chunks_exact(4).map(|p| p[0]).collect()
    }

    #[test]
    fn each_operation_maps_exact_corners_and_alpha() {
        for (op, size, expected) in [
            (Operation::Clockwise, [2, 3], vec![3, 0, 4, 1, 5, 2]),
            (Operation::Counterclockwise, [2, 3], vec![2, 5, 1, 4, 0, 3]),
            (Operation::Mirror, [3, 2], vec![2, 1, 0, 5, 4, 3]),
            (Operation::Flip, [3, 2], vec![3, 4, 5, 0, 1, 2]),
        ] {
            let mut t = Transform::default();
            t.apply(op);
            let copy = t.materialize(&fixture()).unwrap();
            assert_eq!(copy.size, size);
            assert_eq!(labels(&copy), expected);
            for pixel in copy.rgba.chunks_exact(4) {
                assert_eq!(
                    pixel,
                    &[pixel[0], pixel[0] + 10, pixel[0] + 20, pixel[0] * 40]
                );
            }
        }
    }

    #[test]
    fn command_order_is_in_displayed_axes() {
        let mut a = Transform::default();
        a.apply(Operation::Clockwise);
        a.apply(Operation::Mirror);
        assert_eq!(
            labels(&a.materialize(&fixture()).unwrap()),
            [0, 3, 1, 4, 2, 5]
        );
        let mut b = Transform::default();
        b.apply(Operation::Mirror);
        b.apply(Operation::Clockwise);
        assert_eq!(
            labels(&b.materialize(&fixture()).unwrap()),
            [5, 2, 4, 1, 3, 0]
        );
        assert_ne!(a, b);
    }

    #[test]
    fn composition_matrix_matches_sequential_pixel_oracle_and_uv_corners() {
        let ops = [
            Operation::Clockwise,
            Operation::Counterclockwise,
            Operation::Mirror,
            Operation::Flip,
        ];
        for first in ops {
            for second in ops {
                for third in ops {
                    let mut combined = Transform::default();
                    let mut sequential = fixture();
                    for op in [first, second, third] {
                        combined.apply(op);
                        let mut one = Transform::default();
                        one.apply(op);
                        sequential = one.materialize(&sequential).unwrap();
                    }
                    let copy = combined.materialize(&fixture()).unwrap();
                    assert_eq!(copy.size, sequential.size);
                    assert_eq!(copy.rgba, sequential.rgba);
                    for corner in [
                        [0, 0],
                        [copy.size[0] - 1, 0],
                        [0, copy.size[1] - 1],
                        [copy.size[0] - 1, copy.size[1] - 1],
                    ] {
                        let uv = [
                            corner[0] as f32 / (copy.size[0] - 1) as f32,
                            corner[1] as f32 / (copy.size[1] - 1) as f32,
                        ];
                        let rows = combined.uv_rows();
                        let actual = combined.source_pixel(corner, fixture().size);
                        for axis in 0..2 {
                            let mapped =
                                rows[axis][0] * uv[0] + rows[axis][1] * uv[1] + rows[axis][2];
                            assert_eq!(
                                mapped * (fixture().size[axis] - 1) as f32,
                                actual[axis] as f32
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn inverses_and_four_turns_restore_base() {
        let mut t = Transform::default();
        for _ in 0..4 {
            t.apply(Operation::Clockwise);
        }
        assert_eq!(t, Transform::default());
        for op in [Operation::Mirror, Operation::Flip] {
            t.apply(op);
            t.apply(op);
        }
        t.apply(Operation::Clockwise);
        t.apply(Operation::Counterclockwise);
        assert_eq!(t.materialize(&fixture()).unwrap().rgba, fixture().rgba);
    }

    #[test]
    fn invalid_copy_payloads_are_named_resource_failures() {
        for pixels in [
            Pixels {
                size: [usize::MAX, 2],
                rgba: vec![],
                levels: vec![],
            },
            Pixels {
                size: [0, 2],
                rgba: vec![],
                levels: vec![],
            },
            Pixels {
                size: [3, 2],
                rgba: vec![0; 4],
                levels: vec![],
            },
        ] {
            assert!(matches!(
                Transform::default().materialize(&pixels),
                Err(LoadError::ResourceLimit(_))
            ));
        }
    }
}
