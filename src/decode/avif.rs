use super::{LoadError, Pixels};

#[cfg(not(avif_native_built))]
pub(super) fn decode(_: &[u8]) -> Result<Pixels, LoadError> {
    Err(LoadError::UnsupportedVariant(
        "AVIF support not built: libavif/dav1d not found".into(),
    ))
}

#[cfg(avif_native_built)]
pub(super) fn decode(encoded: &[u8]) -> Result<Pixels, LoadError> {
    native::decode(encoded)
}

#[cfg(avif_native_built)]
mod native {
    use super::*;
    use std::ffi::{CStr, c_char, c_void};
    unsafe extern "C" {
        fn glance_avif_decode(
            data: *const u8,
            length: usize,
            out: *mut *mut c_void,
            error: *mut c_char,
            size: usize,
        ) -> i32;
        fn glance_avif_free(handle: *mut c_void);
        fn glance_avif_width(handle: *const c_void) -> u32;
        fn glance_avif_height(handle: *const c_void) -> u32;
        fn glance_avif_stride(handle: *const c_void) -> u32;
        fn glance_avif_pixels(handle: *const c_void) -> *const u8;
        fn glance_avif_icc(handle: *const c_void, length: *mut usize) -> *const u8;
        fn glance_avif_exif(handle: *const c_void, length: *mut usize) -> *const u8;
        fn glance_avif_rotation(handle: *const c_void) -> u32;
        fn glance_avif_mirror(handle: *const c_void) -> u32;
        fn glance_avif_transformed(handle: *const c_void) -> u32;
        fn glance_avif_primaries(handle: *const c_void) -> u32;
        fn glance_avif_transfer(handle: *const c_void) -> u32;
    }
    struct Handle(*mut c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: unique successfully-created shim object, freed exactly once.
            unsafe {
                glance_avif_free(self.0);
            }
        }
    }
    pub(super) fn decode(encoded: &[u8]) -> Result<Pixels, LoadError> {
        let mut raw = std::ptr::null_mut();
        let mut message = [0 as c_char; 512];
        // SAFETY: input stays alive through decoder destruction; out/error are
        // writable and sized. The shim nulls out on every failure and NUL-terminates errors.
        let status = unsafe {
            glance_avif_decode(
                encoded.as_ptr(),
                encoded.len(),
                &mut raw,
                message.as_mut_ptr(),
                message.len(),
            )
        };
        if status != 0 {
            // SAFETY: bounded snprintf in the shim always NUL terminates this buffer.
            let reason = format!(
                "AVIF: {}",
                unsafe { CStr::from_ptr(message.as_ptr()) }.to_string_lossy()
            );
            return Err(match status {
                2 => LoadError::UnsupportedVariant(reason),
                3 => LoadError::ResourceLimit(reason),
                _ => LoadError::Malformed(reason),
            });
        }
        if raw.is_null() {
            return Err(LoadError::Malformed(
                "AVIF: decoder returned no image".into(),
            ));
        }
        let handle = Handle(raw);
        // SAFETY: all getters borrow this live opaque object. Pixel/metadata
        // storage is owned by it and not advanced or mutated before Rust copies it.
        let (w, h, stride, samples, icc, exif, rotation, mirror, transformed, primaries, transfer) = unsafe {
            let w = glance_avif_width(handle.0);
            let h = glance_avif_height(handle.0);
            let stride = glance_avif_stride(handle.0) as usize;
            super::super::checked_size(w, h, super::super::MAX_DIMENSION)?;
            super::super::checked_output_bytes(w, h, 8)?;
            let length = stride
                .checked_mul(h as usize)
                .ok_or_else(|| LoadError::ResourceLimit("AVIF: stride overflow".into()))?;
            if stride < w as usize * 8 || length as u64 > super::super::MAX_DECODED_BYTES {
                return Err(LoadError::ResourceLimit("AVIF: invalid RGB stride".into()));
            }
            let samples = std::slice::from_raw_parts(glance_avif_pixels(handle.0), length);
            let mut n = 0;
            let p = glance_avif_icc(handle.0, &mut n);
            let icc = if n == 0 {
                None
            } else if n > crate::color::MAX_PROFILE_BYTES {
                return Err(LoadError::ResourceLimit("AVIF: ICC exceeds 4 MiB".into()));
            } else {
                Some(std::slice::from_raw_parts(p, n))
            };
            let p = glance_avif_exif(handle.0, &mut n);
            let exif = if n == 0 {
                None
            } else {
                Some(std::slice::from_raw_parts(p, n))
            };
            (
                w,
                h,
                stride,
                samples,
                icc,
                exif,
                glance_avif_rotation(handle.0),
                glance_avif_mirror(handle.0),
                glance_avif_transformed(handle.0),
                glance_avif_primaries(handle.0),
                glance_avif_transfer(handle.0),
            )
        };
        if matches!(transfer, 16 | 18) {
            return Err(LoadError::UnsupportedVariant(
                "AVIF: PQ/HLG HDR transfer is not supported yet".into(),
            ));
        }
        let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
        for row in samples.chunks_exact(stride) {
            rgba.extend(
                row[..w as usize * 8]
                    .chunks_exact(2)
                    .map(|v| u16::from_ne_bytes([v[0], v[1]])),
            );
        }
        let warning = crate::color::normalize16(&mut rgba, icc, false).or_else(|| {
            (icc.is_none() && !(matches!(primaries, 1 | 2) && matches!(transfer, 2 | 13))).then(
                || {
                    format!(
                        "AVIF CICP {primaries}/{transfer}: assuming sRGB (no supported RGB ICC)"
                    )
                },
            )
        });
        let buffer = image::ImageBuffer::<image::Rgba<u16>, _>::from_raw(w, h, rgba)
            .ok_or_else(|| LoadError::Malformed("AVIF: invalid RGBA length".into()))?;
        let mut image = image::DynamicImage::ImageRgba16(buffer);
        // Container transformative properties take precedence over EXIF, never both.
        if transformed != 0 {
            image = match rotation {
                1 => image.rotate270(),
                2 => image.rotate180(),
                3 => image.rotate90(),
                _ => image,
            };
            image = match mirror {
                0 => image.flipv(),
                1 => image.fliph(),
                _ => image,
            };
        } else if let Some(exif) = exif.and_then(image::metadata::Orientation::from_exif_chunk) {
            image.apply_orientation(exif);
        }
        let size = [image.width() as usize, image.height() as usize];
        let rgba16 = image.into_rgba16().into_raw();
        let levels = crate::levels::generate16(size, &rgba16)?;
        Ok(Pixels {
            size,
            rgba16: Some(rgba16),
            levels,
            warning,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_adapter_depth_alpha_stride_and_copy() {
        if !cfg!(avif_native_built) {
            eprintln!("SKIP AVIF real adapter: libavif/dav1d not built");
            return;
        }
        for bytes in [
            include_bytes!("../../assets/test/web/alpha8.avif").as_slice(),
            include_bytes!("../../assets/test/web/alpha10.avif").as_slice(),
            include_bytes!("../../assets/test/web/alpha12.avif").as_slice(),
        ] {
            let p = decode(bytes).unwrap();
            assert_eq!(p.size, [5, 3]);
            let samples = p.rgba16.as_ref().unwrap();
            assert!(samples[0].abs_diff(32768) < 300);
            assert!(samples[1].abs_diff(16384) < 300);
            assert!(samples[2].abs_diff(8192) < 300);
            assert_eq!(samples[3], 0);
            assert_eq!(samples[19], 65535);
            assert!(p.levels[0].half);
            let copy = crate::transform::Transform::default()
                .materialize(&p)
                .unwrap();
            assert_eq!(copy.size, [5, 3]);
            assert_eq!(copy.rgba[3], 0);
            assert_eq!(copy.rgba[19], 255);
        }
    }
    #[test]
    fn truncated_is_named_or_disabled_is_explicit() {
        let error = decode(b"truncated").err().unwrap();
        assert!(error.to_string().contains("AVIF"));
        if !cfg!(avif_native_built) {
            assert!(
                error
                    .to_string()
                    .contains("AVIF support not built: libavif/dav1d not found")
            );
        }
    }
}
