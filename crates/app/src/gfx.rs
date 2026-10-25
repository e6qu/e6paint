//! GPU glue: the vello renderer, an intermediate storage texture, and a blit into egui's
//! render pass.
//!
//! Pipeline shape (verified against vello 0.11 / wgpu 30 docs): vello renders the scene
//! into a `Rgba8Unorm` texture with `STORAGE_BINDING`; a fullscreen-triangle shader then
//! samples it into the egui pass, viewport-clipped to the callback rect. Owning the blit
//! (instead of vello's util blitter) keeps the whole path on APIs verified in this repo.

use std::sync::{Arc, Mutex, MutexGuard};

use eframe::wgpu;
use vello::{AaConfig, RenderParams, Renderer, RendererOptions, Scene};

/// Canvas background behind the board, drawn by vello itself (Miro-light).
fn base_color() -> vello::peniko::Color {
    vello::peniko::Color::from_rgb8(0xF5, 0xF5, 0xF7)
}

/// One frame's CPU-side work product, handed from `update` to the paint callback.
pub struct FrameJob {
    pub scene: Scene,
    pub width: u32,
    pub height: u32,
}

/// Locks `mutex`, surfacing poisoning loudly instead of silently recovering.
pub fn lock_or_log<'a, T>(mutex: &'a Mutex<T>, what: &'static str) -> Option<MutexGuard<'a, T>> {
    match mutex.lock() {
        Ok(guard) => Some(guard),
        Err(poisoned) => {
            log::error!("{what} mutex poisoned: {poisoned}");
            None
        }
    }
}

/// Guard wrapper making the blit path a no-op while disabled, without sprinkling checks.
struct Blitter {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bind_group: Option<wgpu::BindGroup>,
    view: Option<wgpu::TextureView>,
}

/// GPU state owned by the app, driven from the egui wgpu callback.
pub struct Gfx {
    renderer: Option<Renderer>,
    blitter: Option<Blitter>,
    texture: Option<wgpu::Texture>,
    texture_size: (u32, u32),
}

impl Gfx {
    /// Builds the blit pipeline against the surface format egui renders into.
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("e6_blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("e6_blit_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("e6_blit_pl"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("e6_blit_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("e6_blit_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            renderer: None,
            blitter: Some(Blitter {
                pipeline,
                bind_group_layout,
                sampler,
                bind_group: None,
                view: None,
            }),
            texture: None,
            texture_size: (0, 0),
        }
    }

    fn ensure_renderer(&mut self, device: &wgpu::Device) {
        if self.renderer.is_none() {
            match Renderer::new(device, RendererOptions::default()) {
                Ok(renderer) => self.renderer = Some(renderer),
                Err(err) => log::error!("vello renderer creation failed: {err}"),
            }
        }
    }

    fn ensure_texture(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.texture_size == (width, height) && self.texture.is_some() {
            return;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("e6_vello_target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // vello requires Rgba8Unorm + STORAGE_BINDING (compute pipeline writes it).
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        if let Some(blitter) = self.blitter.as_mut() {
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("e6_blit_bg"),
                layout: &blitter.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&blitter.sampler),
                    },
                ],
            });
            blitter.bind_group = Some(bind_group);
            blitter.view = Some(view);
        }
        self.texture = Some(texture);
        self.texture_size = (width, height);
    }

    fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _encoder: &mut wgpu::CommandEncoder,
        job: &FrameJob,
    ) {
        self.ensure_renderer(device);
        self.ensure_texture(device, job.width, job.height);
        let (Some(renderer), Some(blitter)) = (self.renderer.as_mut(), self.blitter.as_ref())
        else {
            return;
        };
        let Some(view) = blitter.view.as_ref() else {
            return;
        };
        if let Err(err) = renderer.render_to_texture(
            device,
            queue,
            &job.scene,
            view,
            &RenderParams {
                base_color: base_color(),
                width: job.width,
                height: job.height,
                antialiasing_method: AaConfig::Area,
            },
        ) {
            log::error!("vello render_to_texture failed: {err}");
        }
    }

    fn blit(
        &mut self,
        render_pass: &mut wgpu::RenderPass<'static>,
        viewport: eframe::egui::epaint::ViewportInPixels,
    ) {
        let Some(blitter) = self.blitter.as_ref() else {
            return;
        };
        let Some(bind_group) = blitter.bind_group.as_ref() else {
            return;
        };
        render_pass.set_viewport(
            viewport.left_px as f32,
            viewport.top_px as f32,
            viewport.width_px as f32,
            viewport.height_px as f32,
            0.0,
            1.0,
        );
        render_pass.set_scissor_rect(
            viewport.left_px as u32,
            viewport.top_px as u32,
            viewport.width_px as u32,
            viewport.height_px as u32,
        );
        render_pass.set_pipeline(&blitter.pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

/// egui wgpu paint callback bridging one [`FrameJob`] through [`Gfx`].
pub struct VelloCallback {
    pub gfx: Arc<Mutex<Gfx>>,
    pub job: Arc<Mutex<Option<FrameJob>>>,
}

impl eframe::egui_wgpu::CallbackTrait for VelloCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &eframe::egui_wgpu::ScreenDescriptor,
        // vello's render_to_texture encodes and submits internally; egui's encoder is unused.
        _encoder: &mut wgpu::CommandEncoder,
        _callback_resources: &mut eframe::egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let job = lock_or_log(&self.job, "frame job").and_then(|mut slot| slot.take());
        let Some(job) = job else { return Vec::new() };
        if let Some(mut gfx) = lock_or_log(&self.gfx, "gfx") {
            gfx.render(device, queue, _encoder, &job);
        }
        Vec::new()
    }

    fn paint(
        &self,
        info: eframe::egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        _callback_resources: &eframe::egui_wgpu::CallbackResources,
    ) {
        if let Some(mut gfx) = lock_or_log(&self.gfx, "gfx") {
            gfx.blit(render_pass, info.viewport_in_pixels());
        }
    }
}

const BLIT_WGSL: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
};

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VsOut {
    let xy = vec2f(f32(idx & 1u) * 4.0 - 1.0, f32(idx >> 1u) * 4.0 - 1.0);
    var out: VsOut;
    out.pos = vec4f(xy, 0.0, 1.0);
    out.uv = vec2f(xy.x * 0.5 + 0.5, 0.5 - xy.y * 0.5);
    return out;
}

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    return textureSample(tex, samp, in.uv);
}
"#;
