use crate::{
    decode::{LoadError, Pixels},
    levels,
    settings::Background,
    tiles::{self, Key},
    transform::Transform,
};
use eframe::{
    egui,
    egui_wgpu::{self, wgpu},
};
use std::{
    collections::HashMap,
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Waker},
};
use wgpu::util::DeviceExt;

pub struct Prepared {
    pub paint: egui::PaintCallback,
    pub coverage: tiles::CoverageReady,
}

pub struct Image {
    pub size: [usize; 2],
    pub pixels: Arc<Pixels>,
    pub has_transparency: bool,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    core: usize,
    resident: Mutex<Residency>,
    uploads: Arc<Mutex<Uploads>>,
}

// One tile (at most 8 MiB, plus its CPU gutter copy) may be in flight for
// the entire viewer, including cancelled selections. Never accumulate driver
// staging by submitting faster than the GPU retires it.
#[derive(Default)]
pub struct Uploads {
    frame: Option<u64>,
    submitted_frame: Option<u64>,
    pending: Option<Arc<AtomicBool>>,
    failure: Arc<Mutex<Option<String>>>,
}

impl Uploads {
    pub fn new(state: Option<&egui_wgpu::RenderState>) -> Arc<Mutex<Self>> {
        let uploads = Self::default();
        if let Some(state) = state {
            let failure = Arc::clone(&uploads.failure);
            state
                .device
                .set_device_lost_callback(move |reason, message| {
                    *failure.lock().expect("device failure lock poisoned") =
                        Some(format!("Device lost ({reason:?}): {message}"));
                });
        }
        Arc::new(Mutex::new(uploads))
    }

    fn maintain(&mut self, frame: u64, device: &wgpu::Device) -> Result<(), LoadError> {
        if self.frame != Some(frame) {
            self.frame = Some(frame);
            if let Err(error) = checked(device, || {
                device.poll(wgpu::PollType::Poll).map_err(|e| {
                    LoadError::DisplayUnavailable(format!("Tile upload failed: {e}"))
                })?;
                Ok(())
            }) {
                let message = match &error {
                    LoadError::DisplayUnavailable(message) => message.clone(),
                    _ => error.to_string(),
                };
                *self.failure.lock().expect("device failure lock poisoned") = Some(message);
                return Err(error);
            }
        }
        self.check_failure()
    }

    fn check_failure(&self) -> Result<(), LoadError> {
        if let Some(error) = &*self.failure.lock().expect("device failure lock poisoned") {
            return Err(LoadError::DisplayUnavailable(error.clone()));
        }
        Ok(())
    }

    pub(crate) fn coverage(
        &self,
        frame: u64,
        visible: &[Key],
        complete: impl Fn(&Key) -> bool,
    ) -> Result<Option<tiles::CoverageReady>, LoadError> {
        self.check_failure()?;
        if self.submitted_frame == Some(frame) {
            return Ok(None);
        }
        Ok(tiles::ready(visible, complete))
    }

    #[cfg(test)]
    pub(crate) fn inject_failure(&self) {
        *self.failure.lock().unwrap() = Some("Injected upload/device failure".into());
    }

    pub(crate) fn available(&self, frame: u64) -> bool {
        self.submitted_frame != Some(frame)
            && self
                .pending
                .as_ref()
                .is_none_or(|done| done.load(Ordering::Acquire))
    }

    pub(crate) fn submitted(&mut self, frame: u64, done: Arc<AtomicBool>) {
        self.submitted_frame = Some(frame);
        self.pending = Some(done);
    }
}

struct Resident {
    texture: wgpu::TextureView,
    bytes: usize,
    age: u64,
    done: Arc<AtomicBool>,
}
#[derive(Default)]
struct Residency {
    tiles: HashMap<Key, Resident>,
    clock: u64,
}

// Native wgpu resolves scoped creation/write errors synchronously. Never assume
// success if an error future is pending (e.g. a different backend).
fn checked<T>(
    device: &wgpu::Device,
    operation: impl FnOnce() -> Result<T, LoadError>,
) -> Result<T, LoadError> {
    let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let result = operation();
    let mut cx = Context::from_waker(Waker::noop());
    let mut error = None;
    for scope in [validation, internal, oom] {
        let future = scope.pop();
        match std::pin::pin!(future).as_mut().poll(&mut cx) {
            Poll::Ready(None) => {}
            Poll::Ready(Some(e)) => error = Some(e.to_string()),
            Poll::Pending => error = Some("Device error check did not complete".into()),
        }
    }
    if let Some(error) = error {
        Err(LoadError::DisplayUnavailable(error))
    } else {
        result
    }
}

impl Image {
    // Only creates the renderer. The caller must stage visible coverage before
    // admission; no texture-size rejection of a CPU-admitted image.
    pub fn upload(
        state: &egui_wgpu::RenderState,
        pixels: Arc<Pixels>,
        uploads: Arc<Mutex<Uploads>>,
    ) -> Result<Arc<Self>, LoadError> {
        if pixels.levels.is_empty() {
            return Err(LoadError::DisplayUnavailable(
                "Missing normalized levels".into(),
            ));
        }
        let core = tiles::core(state.device.limits().max_texture_dimension_2d)?;
        // Scan once for this ready resident, not in the per-frame callback.
        let has_transparency = pixels.has_transparency();
        let device = &state.device;
        checked(device, || {
            let mut entries = vec![wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }];
            for binding in [1, 3] {
                entries.push(wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                });
            }
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("image bindings"),
                entries: &entries,
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("image pipeline layout"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("image shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("image.wgsl").into()),
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("tiled image pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(if state.target_format.is_srgb() {
                        "linear"
                    } else {
                        "encoded"
                    }),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: state.target_format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
            let linear = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("reduction sampler"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });
            let nearest = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("pixel inspection sampler"),
                ..Default::default()
            });
            Ok(Arc::new(Self {
                size: pixels.svg.as_ref().map_or(pixels.size, |svg| svg.size),
                pixels,
                has_transparency,
                pipeline,
                layout,
                linear,
                nearest,
                core,
                resident: Mutex::new(Residency::default()),
                uploads,
            }))
        })
    }

    fn stage(
        &self,
        state: &egui_wgpu::RenderState,
        visible: &[Key],
        adjacent: &[Key],
        frame: u64,
    ) -> Result<Option<tiles::CoverageReady>, LoadError> {
        let mut uploads = self.uploads.lock().expect("upload budget lock poisoned");
        uploads.maintain(frame, &state.device)?;
        let bytes = |key: &Key| {
            let level = &self.pixels.levels[key.level];
            tiles::tile(level.size, *key, self.core).bytes_at_depth(level.bytes_per_pixel())
        };
        if visible.iter().map(bytes).sum::<usize>() > tiles::RESIDENT_BYTES {
            return Err(LoadError::ResourceLimit(
                "Visible GPU tiles exceed 128 MiB residency".into(),
            ));
        }
        let mut cache = self.resident.lock().expect("GPU residency lock poisoned");
        cache.clock = cache.clock.saturating_add(1);
        let age = cache.clock;
        for key in visible {
            if let Some(tile) = cache.tiles.get_mut(key) {
                tile.age = age;
            }
        }
        let mut protected = visible.to_vec();
        let mut required = visible.iter().map(bytes).sum::<usize>();
        for key in adjacent {
            if required + bytes(key) <= tiles::RESIDENT_BYTES {
                protected.push(*key);
                required += bytes(key);
            }
        }
        for key in &protected {
            if cache.tiles.contains_key(key) {
                continue;
            }
            if !uploads.available(frame) {
                break;
            }
            let needed = bytes(key);
            while cache.tiles.values().map(|t| t.bytes).sum::<usize>() + needed
                > tiles::RESIDENT_BYTES
            {
                let recency: Vec<_> = cache.tiles.iter().map(|(k, t)| (*k, t.age)).collect();
                let victim = tiles::eviction(&recency, &protected)
                    .ok_or_else(|| LoadError::ResourceLimit("GPU residency exhausted".into()))?;
                cache.tiles.remove(&victim);
            }
            let level = &self.pixels.levels[key.level];
            let tile = tiles::tile(level.size, *key, self.core);
            let data = tiles::gutter(level, tile)?;
            if data.len() > tiles::STAGING_BYTES {
                return Err(LoadError::ResourceLimit(
                    "GPU tile staging exceeds 16 MiB".into(),
                ));
            }
            // Submission is asynchronous. The coverage witness is withheld until
            // a later frame polls and observes this submission's completion.
            let done = Arc::new(AtomicBool::new(false));
            let texture = checked(&state.device, || {
                let texture = state.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("linear-premultiplied sRGB tile with gutter"),
                    size: wgpu::Extent3d {
                        width: (tile.size[0] + 2) as u32,
                        height: (tile.size[1] + 2) as u32,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: if level.half {
                        wgpu::TextureFormat::Rgba16Float
                    } else {
                        wgpu::TextureFormat::Rgba8UnormSrgb
                    },
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                state.queue.write_texture(
                    texture.as_image_copy(),
                    &data,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(
                            (tile.size[0] + 2) as u32 * level.bytes_per_pixel() as u32,
                        ),
                        rows_per_image: Some((tile.size[1] + 2) as u32),
                    },
                    texture.size(),
                );
                state.queue.submit([]);
                let completed = Arc::clone(&done);
                state.queue.on_submitted_work_done(move || {
                    completed.store(true, Ordering::Release);
                });
                Ok(texture.create_view(&Default::default()))
            });
            uploads.submitted(frame, Arc::clone(&done));
            let texture = texture?;
            cache.tiles.insert(
                *key,
                Resident {
                    texture,
                    bytes: needed,
                    age,
                    done,
                },
            );
        }
        uploads.coverage(frame, visible, |key| {
            cache
                .tiles
                .get(key)
                .is_some_and(|tile| tile.done.load(Ordering::Acquire))
        })
    }

    // Keep the per-paint framing and appearance explicit at the callback boundary.
    #[allow(clippy::too_many_arguments)]
    pub fn callback(
        self: &Arc<Self>,
        state: &egui_wgpu::RenderState,
        canvas: egui::Rect,
        rect: egui::Rect,
        transform: Transform,
        physical_scale: f32,
        frame: u64,
        background: Background,
    ) -> Result<Option<Prepared>, LoadError> {
        // View framing remains in intrinsic SVG units; levels are in raster pixels.
        let physical_scale = if self.pixels.svg.is_some() {
            physical_scale / self.pixels.svg_scale
        } else {
            physical_scale
        };
        let (a, b, blend) = levels::select(&self.pixels.levels, physical_scale);
        let visible = source_visible(canvas, rect, transform);
        let first = tiles::coverage(self.pixels.levels[a].size, a, self.core, visible);
        let second = if a == b {
            first.clone()
        } else {
            tiles::coverage(self.pixels.levels[b].size, b, self.core, visible)
        };
        let mut keys = first.clone();
        if a != b {
            keys.extend(&second);
        }
        let mut adjacent = tiles::adjacent(self.pixels.levels[a].size, self.core, &first);
        if a != b {
            adjacent.extend(tiles::adjacent(
                self.pixels.levels[b].size,
                self.core,
                &second,
            ));
        }
        let Some(coverage) = self.stage(state, &keys, &adjacent, frame)? else {
            return Ok(None);
        };
        checked(&state.device, || {
            let cache = self.resident.lock().expect("GPU residency lock poisoned");
            let mut bindings = Vec::new();
            for ka in &first {
                for kb in &second {
                    if a == b && ka != kb {
                        continue;
                    }
                    let ta = tiles::tile(self.pixels.levels[a].size, *ka, self.core);
                    let tb = tiles::tile(self.pixels.levels[b].size, *kb, self.core);
                    let Some(domain) = tiles::intersection(
                        ta.domain(self.pixels.levels[a].size),
                        tb.domain(self.pixels.levels[b].size),
                    ) else {
                        continue;
                    };
                    let screen = screen_domain(domain, rect, transform);
                    let bounds = [
                        2.0 * (screen.left() - canvas.left()) / canvas.width() - 1.0,
                        1.0 - 2.0 * (screen.top() - canvas.top()) / canvas.height(),
                        2.0 * (screen.right() - canvas.left()) / canvas.width() - 1.0,
                        1.0 - 2.0 * (screen.bottom() - canvas.top()) / canvas.height(),
                    ];
                    let rows = |tile: tiles::Tile, size: [usize; 2]| -> [[f32; 4]; 2] {
                        let inverse = transform.uv_rows();
                        std::array::from_fn(|axis| {
                            let extent = domain[axis + 2] - domain[axis];
                            let scale = size[axis] as f32 / (tile.size[axis] + 2) as f32;
                            [
                                inverse[axis][0] * extent * scale,
                                inverse[axis][1] * extent * scale,
                                ((domain[axis] + inverse[axis][2] * extent) * size[axis] as f32
                                    - tile.origin[axis] as f32
                                    + 1.0)
                                    / (tile.size[axis] + 2) as f32,
                                0.0,
                            ]
                        })
                    };
                    let ra = rows(ta, self.pixels.levels[a].size);
                    let rb = rows(tb, self.pixels.levels[b].size);
                    let mix = blend_uniform(blend, background);
                    let bytes: Vec<u8> = bounds
                        .iter()
                        .chain(ra.iter().flatten())
                        .chain(rb.iter().flatten())
                        .chain(mix.iter())
                        .flat_map(|n| n.to_ne_bytes())
                        .collect();
                    let buffer =
                        state
                            .device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("tile framing"),
                                contents: &bytes,
                                usage: wgpu::BufferUsages::UNIFORM,
                            });
                    bindings.push(state.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("tile frame"),
                        layout: &self.layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(
                                    &cache.tiles[ka].texture,
                                ),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(
                                    &cache.tiles[kb].texture,
                                ),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(
                                    if physical_scale >= 1.0 {
                                        &self.nearest
                                    } else {
                                        &self.linear
                                    },
                                ),
                            },
                        ],
                    }));
                }
            }
            Ok(Some(Prepared {
                coverage,
                paint: egui_wgpu::Callback::new_paint_callback(
                    canvas,
                    Paint {
                        image: Arc::clone(self),
                        bindings,
                    },
                ),
            }))
        })
    }
}

// Matches Mapping.blend in image.wgsl: x = level blend, yzw = linear backdrop RGB.
// All six uniform members are vec4<f32>, packed sequentially at 16-byte boundaries.
fn blend_uniform(blend: f32, background: Background) -> [f32; 4] {
    let value = background.linear();
    [blend, value, value, value]
}

fn source_visible(canvas: egui::Rect, rect: egui::Rect, transform: Transform) -> [f32; 4] {
    let clipped = canvas.intersect(rect);
    if !clipped.is_positive() {
        return [0.0; 4];
    }
    let rows = transform.uv_rows();
    let mut out = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for p in [
        clipped.left_top(),
        clipped.right_top(),
        clipped.left_bottom(),
        clipped.right_bottom(),
    ] {
        let uv = [
            (p.x - rect.left()) / rect.width(),
            (p.y - rect.top()) / rect.height(),
        ];
        for axis in 0..2 {
            let v = rows[axis][0] * uv[0] + rows[axis][1] * uv[1] + rows[axis][2];
            out[axis] = out[axis].min(v);
            out[axis + 2] = out[axis + 2].max(v);
        }
    }
    out
}

fn screen_domain(domain: [f32; 4], rect: egui::Rect, transform: Transform) -> egui::Rect {
    let rows = transform.uv_rows();
    let forward = |s: [f32; 2]| {
        egui::pos2(
            rect.left()
                + rect.width()
                    * (rows[0][0] * (s[0] - rows[0][2]) + rows[1][0] * (s[1] - rows[1][2])),
            rect.top()
                + rect.height()
                    * (rows[0][1] * (s[0] - rows[0][2]) + rows[1][1] * (s[1] - rows[1][2])),
        )
    };
    let a = forward([domain[0], domain[1]]);
    let b = forward([domain[2], domain[3]]);
    egui::Rect::from_min_max(a.min(b), a.max(b))
}

struct Paint {
    image: Arc<Image>,
    bindings: Vec<wgpu::BindGroup>,
}
impl egui_wgpu::CallbackTrait for Paint {
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        _resources: &egui_wgpu::CallbackResources,
    ) {
        pass.set_pipeline(&self.image.pipeline);
        for binding in &self.bindings {
            pass.set_bind_group(0, binding, &[]);
            pass.draw(0..6, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backdrop_uniform_preserves_black_and_linearizes_white_and_grey() {
        assert_eq!(
            blend_uniform(0.25, Background::Black),
            [0.25, 0.006048833, 0.006048833, 0.006048833]
        );
        assert_eq!(
            blend_uniform(0.75, Background::White),
            [0.75, 1.0, 1.0, 1.0]
        );
        let grey = ((128.0_f32 / 255.0 + 0.055) / 1.055).powf(2.4);
        assert_eq!(
            blend_uniform(0.5, Background::Grey),
            [0.5, grey, grey, grey]
        );
    }

    #[test]
    fn callback_obeys_wgpu_thread_contract() {
        fn assert_callback<T: egui_wgpu::CallbackTrait + Send + Sync>() {}
        assert_callback::<Paint>();
    }
    #[test]
    fn transformed_clipped_coverage_matches_source_tile_domain() {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 600.0));
        let domain = [0.7, 0.1, 0.9, 0.3];
        let mut transform = Transform::default();
        for op in [
            crate::transform::Operation::Clockwise,
            crate::transform::Operation::Mirror,
            crate::transform::Operation::Flip,
        ] {
            transform.apply(op);
            let screen = screen_domain(domain, rect, transform);
            let recovered = source_visible(screen, rect, transform);
            for i in 0..4 {
                assert!((domain[i] - recovered[i]).abs() < 0.000001);
            }
        }
    }
}
