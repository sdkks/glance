use std::sync::Arc;

use eframe::{
    egui,
    egui_wgpu::{self, wgpu},
};
use wgpu::util::DeviceExt;

use crate::decode::{LoadError, Pixels};

pub struct Image {
    pub size: [usize; 2],
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    texture: wgpu::TextureView,
    sampler: wgpu::Sampler,
}

impl Image {
    pub fn upload(state: &egui_wgpu::RenderState, pixels: Pixels) -> Result<Arc<Self>, LoadError> {
        let width = u32::try_from(pixels.size[0])
            .map_err(|_| LoadError::ResourceLimit("GPU width".into()))?;
        let height = u32::try_from(pixels.size[1])
            .map_err(|_| LoadError::ResourceLimit("GPU height".into()))?;
        let row = width
            .checked_mul(4)
            .ok_or_else(|| LoadError::ResourceLimit("GPU stride".into()))?;
        let bytes = usize::try_from(u64::from(row) * u64::from(height))
            .map_err(|_| LoadError::ResourceLimit("GPU bytes".into()))?;
        if width == 0
            || height == 0
            || pixels.rgba.len() != bytes
            || width.max(height) > state.device.limits().max_texture_dimension_2d
        {
            return Err(LoadError::ResourceLimit(
                "Invalid GPU image dimensions or payload".into(),
            ));
        }
        let device = &state.device;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("image RGBA8"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        state.queue.write_texture(
            texture.as_image_copy(),
            &pixels.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
            texture.size(),
        );
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("image bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
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
            label: Some("image pipeline"),
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("image linear sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(Arc::new(Self {
            size: pixels.size,
            pipeline,
            layout,
            texture: texture.create_view(&Default::default()),
            sampler,
        }))
    }

    pub fn callback(
        self: &Arc<Self>,
        device: &wgpu::Device,
        canvas: egui::Rect,
        rect: egui::Rect,
    ) -> egui::PaintCallback {
        let bounds = [
            2.0 * (rect.left() - canvas.left()) / canvas.width() - 1.0,
            1.0 - 2.0 * (rect.top() - canvas.top()) / canvas.height(),
            2.0 * (rect.right() - canvas.left()) / canvas.width() - 1.0,
            1.0 - 2.0 * (rect.bottom() - canvas.top()) / canvas.height(),
        ];
        let bytes: Vec<u8> = bounds.iter().flat_map(|n| n.to_ne_bytes()).collect();
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("image framing"),
            contents: &bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("image frame bindings"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.texture),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        egui_wgpu::Callback::new_paint_callback(
            canvas,
            Paint {
                image: Arc::clone(self),
                bindings,
            },
        )
    }
}

struct Paint {
    image: Arc<Image>,
    bindings: wgpu::BindGroup,
}

impl egui_wgpu::CallbackTrait for Paint {
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        _resources: &egui_wgpu::CallbackResources,
    ) {
        pass.set_pipeline(&self.image.pipeline);
        pass.set_bind_group(0, &self.bindings, &[]);
        pass.draw(0..6, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_obeys_wgpu_thread_contract() {
        fn assert_callback<T: egui_wgpu::CallbackTrait + Send + Sync>() {}
        assert_callback::<Paint>();
    }
}
