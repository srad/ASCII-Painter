//! Image-only compositor. Rune retains layout, clipping, input, text and painter order.
use crate::{app::State, viewer::ViewerImage};
use rune::compose::{Compositor, DeviceKey, Frame};
use std::collections::HashMap;
use std::sync::atomic::Ordering;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    rect: [f32; 4],
    target: [f32; 4],
    clip: [f32; 4],
    fade: [u32; 4],
    encoding: [f32; 4],
}
struct Texture {
    id: u64,
    group: wgpu::BindGroup,
    params: wgpu::Buffer,
}
struct Gpu {
    key: DeviceKey,
    layout: wgpu::BindGroupLayout,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    images: HashMap<u32, Texture>,
}
#[derive(Default)]
pub struct ViewerCompositor {
    gpu: Option<Gpu>,
}

impl Gpu {
    fn new(device: &wgpu::Device, key: DeviceKey) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewer bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    count: None,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    count: None,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    count: None,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                },
            ],
        });
        Self { key, layout, pipelines: HashMap::new(), images: HashMap::new() }
    }
    fn pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        self.pipelines.entry(format).or_insert_with(|| {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("viewer shader"),
                source: wgpu::ShaderSource::Wgsl(
                    format!("{}\n{}", rune::render::CLIP_WGSL, include_str!("viewer.wgsl")).into(),
                ),
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(&self.layout)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("viewer pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        });
    }
    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, mark: u32, image: &ViewerImage) {
        if self.images.get(&mark).is_some_and(|t| t.id == image.id) {
            return;
        }
        // A recreated window may move to a device with a smaller texture limit.
        // Its existing pyramid already contains a safe fallback; never over-allocate.
        let limit = device.limits().max_texture_dimension_2d;
        let first = image.levels.iter().position(|p| p.width() <= limit && p.height() <= limit).unwrap();
        let levels = &image.levels[first..];
        let base = &levels[0];
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("viewer image"),
            size: wgpu::Extent3d { width: base.width(), height: base.height(), depth_or_array_layers: 1 },
            mip_level_count: levels.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, pixels) in levels.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                pixels.as_raw(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pixels.width() * 4),
                    rows_per_image: Some(pixels.height()),
                },
                wgpu::Extent3d { width: pixels.width(), height: pixels.height(), depth_or_array_layers: 1 },
            );
        }
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ASCII nearest magnification, filtered mip minification"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewer geometry"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view = texture.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
            ],
        });
        self.images.insert(mark, Texture { id: image.id, group, params });
    }
}

impl Compositor<State> for ViewerCompositor {
    fn render(&mut self, f: &mut Frame<'_, State>) {
        let limit = f.device.limits().max_texture_dimension_2d.min(8192);
        f.rt.state.texture_limit.store(limit, Ordering::Relaxed);
        if self.gpu.as_ref().is_none_or(|g| g.key != f.device_key()) {
            log::info!("Initialized viewer GPU pipeline (max_texture_dimension_2d={limit}, format={:?})", f.format);
            self.gpu = Some(Gpu::new(f.device, f.device_key()));
        }
        let gpu = self.gpu.as_mut().unwrap();
        gpu.pipeline(f.device, f.format);
        gpu.pipeline(f.device, wgpu::TextureFormat::Rgba16Float);
        for (mark, image) in [(1, &f.rt.state.preview.original), (2, &f.rt.state.preview.ascii)] {
            if let Some(image) = image {
                gpu.upload(f.device, f.queue, mark, image);
            } else {
                gpu.images.remove(&mark);
            }
        }
        let clear = f.rt.clear_color();
        f.ui.prepare(f.device, f.queue, &mut f.rt.text.atlas, &f.rt.instances, f.size);
        f.ui.draw_with(f.encoder, f.target, clear, &mut |pass, site| {
            let Some(image) = gpu.images.get(&site.mark) else { return };
            let Some(pipeline) = gpu.pipelines.get(&site.format) else { return };
            let params = Params {
                rect: site.instance.rect,
                target: [site.size[0] as f32, site.size[1] as f32, site.origin[0], site.origin[1]],
                clip: site.instance.clip,
                fade: site.instance.fade,
                encoding: [if site.srgb_blend { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
            };
            f.queue.write_buffer(&image.params, 0, bytemuck::bytes_of(&params));
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &image.group, &[]);
            pass.draw(0..6, 0..1);
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn viewer_shader_parses_and_validates_with_runes_clip_contract() {
        let source = format!("{}\n{}", rune::render::CLIP_WGSL, include_str!("viewer.wgsl"));
        let shader = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::SHADER_FLOAT16_IN_FLOAT32,
        )
        .validate(&shader)
        .unwrap();
    }
}
