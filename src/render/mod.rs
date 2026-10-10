// wgpu-Renderer (auf dem Mac über Metal): zeichnet den Ebenenstapel in eine Textur je Ansicht
// (Live-Vorschau in der Steuerung und Beamer-Ausgabe). Inhalte (Bilder, Videos, Kamera, Milkdrop,
// Effekte, Masken) liegen als Texturen im gemeinsamen Speicher und werden nur einmal pro Bild erzeugt.

#[cfg(test)]
mod gpu_tests;
pub mod masks;
pub mod testgrid;

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use wgpu::util::DeviceExt;

use crate::effects::{EFFECT_VERT_WGSL, EffectDef, EffectUniforms, parse_effect};
use crate::geometry::{surface_inverse, triangle_matrix, invert, Mat3};
use crate::model::{BlendMode, Quad, Shape};

/// Format aller Bildpuffer: Mischen im Gamma-Raum wie im Browser (WebGL)
pub const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const FEEDBACK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const UNIFORM_STRIDE: u64 = 256;

#[derive(Clone)]
pub struct TexEntry {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

/// Was auf eine Fläche kommt
#[derive(Clone, Debug)]
pub enum Source {
    /// Textur aus dem Speicher (Schlüssel)
    Texture(String),
    Color([f32; 3]),
    /// nur Raster
    None,
}

#[derive(Clone, Debug)]
pub struct GridDraw {
    pub cols: f32,
    pub rows: f32,
    /// Linienstärke in Ausgabepixeln
    pub thickness: f32,
    pub color: [f32; 3],
    pub diagonals: bool,
}

#[derive(Clone, Debug)]
pub struct SurfaceDraw {
    /// Flächen-ID (Mesh-Puffer)
    pub key: String,
    pub shape: Shape,
    pub corners: Quad,
    /// Dreiecksnetz (nur Mesh): pos.xy, uv.xy je Eckpunkt
    pub mesh: Option<std::sync::Arc<Vec<[f32; 4]>>>,
    pub source: Source,
    pub opacity: f32,
    pub blend: BlendMode,
    pub fit_scale: [f32; 2],
    /// Maskentextur (Schlüssel) im Inhaltsraum
    pub mask: Option<String>,
    pub grid: Option<GridDraw>,
}

#[derive(Clone, Debug)]
pub enum DrawItem {
    Surface(SurfaceDraw),
    /// Masken-Ebene: subtract = Inneres ausschneiden, sonst nur Inneres behalten
    Mask { texture: String, subtract: bool },
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default)]
struct SurfaceUniforms {
    inv0: [f32; 4],
    inv1: [f32; 4],
    inv2: [f32; 4],
    color: [f32; 4],
    fit: [f32; 4],
    grid: [f32; 4],
    grid_color: [f32; 4],
    flags: [f32; 4],
}

/// Zielbild einer Ansicht (Vorschau oder Ausgabe)
pub struct ViewTarget {
    #[cfg_attr(not(test), allow(dead_code))]
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// sRGB-Sicht derselben Textur für egui (Werte bleiben unverändert)
    pub srgb_view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    uniforms: wgpu::Buffer,
    uniform_cap: u64,
    vertices: wgpu::Buffer,
    vertex_cap: u64,
    pub egui_id: Option<egui::TextureId>,
}

struct EffectTarget {
    tex: Vec<TexEntry>,
    cur: usize,
    uniforms: wgpu::Buffer,
    effect: String,
}

struct MeshBuf {
    data: std::sync::Arc<Vec<[f32; 4]>>,
    buffer: wgpu::Buffer,
    count: u32,
}

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    textures: HashMap<String, TexEntry>,
    surface_layout: wgpu::BindGroupLayout,
    surface_pipelines: HashMap<BlendMode, wgpu::RenderPipeline>,
    full_layout: wgpu::BindGroupLayout,
    mask_subtract: wgpu::RenderPipeline,
    mask_keep: wgpu::RenderPipeline,
    dim: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    white: TexEntry,
    black: TexEntry,
    effect_layout: wgpu::BindGroupLayout,
    effect_pipeline_layout: wgpu::PipelineLayout,
    effect_vs: wgpu::ShaderModule,
    effect_pipelines: HashMap<String, Option<wgpu::RenderPipeline>>,
    effect_targets: HashMap<String, EffectTarget>,
    meshes: HashMap<String, MeshBuf>,
}

fn blend_state(mode: BlendMode) -> wgpu::BlendState {
    use wgpu::{BlendComponent as C, BlendFactor as F, BlendOperation as O};
    let comp = |s, d| C { src_factor: s, dst_factor: d, operation: O::Add };
    match mode {
        BlendMode::Normal => wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        BlendMode::Add => wgpu::BlendState { color: comp(F::One, F::One), alpha: comp(F::One, F::One) },
        BlendMode::Screen => wgpu::BlendState { color: comp(F::One, F::OneMinusSrc), alpha: comp(F::One, F::OneMinusSrcAlpha) },
        // Ergebnis = Quelle × Ziel + Ziel × (1 − Quell-Alpha)
        BlendMode::Multiply => wgpu::BlendState { color: comp(F::Dst, F::OneMinusSrcAlpha), alpha: comp(F::Dst, F::OneMinusSrcAlpha) },
    }
}

fn texture_entry(device: &wgpu::Device, label: &str, w: u32, h: u32, format: wgpu::TextureFormat, mips: u32, usage: wgpu::TextureUsages) -> TexEntry {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    TexEntry { texture, view, width: w, height: h }
}

fn mip_count(w: u32, h: u32) -> u32 {
    32 - w.max(h).max(1).leading_zeros()
}

/// Halbiert ein RGBA-Bild (2×2-Mittel, für vormultipliziertes Alpha korrekt)
fn downsample(src: &[u8], w: u32, h: u32) -> (Vec<u8>, u32, u32) {
    let nw = (w / 2).max(1);
    let nh = (h / 2).max(1);
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        for x in 0..nw {
            for c in 0..4 {
                let mut sum = 0u32;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (x * 2 + dx).min(w - 1);
                    let sy = (y * 2 + dy).min(h - 1);
                    sum += src[((sy * w + sx) * 4 + c) as usize] as u32;
                }
                out[((y * nw + x) * 4 + c) as usize] = ((sum + 2) / 4) as u8;
            }
        }
    }
    (out, nw, nh)
}

impl Renderer {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let surface_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("surface"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("surface.wgsl"))),
        });
        let full_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("full"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("full.wgsl"))),
        });
        let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let surface_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("surface"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<SurfaceUniforms>() as u64),
                    },
                    count: None,
                },
                tex_entry(1),
                tex_entry(2),
                sampler_entry(3),
            ],
        });
        let full_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("full"),
            entries: &[tex_entry(0), sampler_entry(1)],
        });
        let surface_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("surface"),
            bind_group_layouts: &[Some(&surface_layout)],
            immediate_size: 0,
        });
        let full_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("full"),
            bind_group_layouts: &[Some(&full_layout)],
            immediate_size: 0,
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: 16,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2],
        };
        let mut surface_pipelines = HashMap::new();
        for mode in BlendMode::ALL {
            let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("surface"),
                layout: Some(&surface_pl),
                vertex: wgpu::VertexState {
                    module: &surface_module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(vertex_layout.clone())],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &surface_module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: TARGET_FORMAT,
                        blend: Some(blend_state(mode)),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
            surface_pipelines.insert(mode, p);
        }

        let full_pipeline = |entry: &str, blend: wgpu::BlendState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&full_pl),
                vertex: wgpu::VertexState {
                    module: &full_module,
                    entry_point: Some("vs_full"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &full_module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: TARGET_FORMAT, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        use wgpu::{BlendComponent as C, BlendFactor as F, BlendOperation as O};
        let mul = |d| {
            let c = C { src_factor: F::Zero, dst_factor: d, operation: O::Add };
            wgpu::BlendState { color: c, alpha: c }
        };
        // Ziel × (1 − Maske) beim Ausschneiden, sonst Ziel × Maske; Gesamthelligkeit: Ziel × Konstante
        let mask_subtract = full_pipeline("fs_mask", mul(F::OneMinusSrcAlpha));
        let mask_keep = full_pipeline("fs_mask", mul(F::SrcAlpha));
        let dim = full_pipeline("fs_solid", mul(F::Constant));

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let white = texture_entry(&device, "white", 1, 1, wgpu::TextureFormat::Rgba8Unorm, 1, usage);
        let black = texture_entry(&device, "black", 1, 1, wgpu::TextureFormat::Rgba8Unorm, 1, usage);
        for (t, px) in [(&white, [255u8; 4]), (&black, [0, 0, 0, 0])] {
            queue.write_texture(
                t.texture.as_image_copy(),
                &px,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: None },
                wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            );
        }

        let effect_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("effect"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                tex_entry(1),
                tex_entry(2),
                sampler_entry(3),
            ],
        });
        let effect_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("effect"),
            bind_group_layouts: &[Some(&effect_layout)],
            immediate_size: 0,
        });
        let effect_vs = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("effect-vs"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(EFFECT_VERT_WGSL)),
        });

        let mut r = Self {
            device,
            queue,
            textures: HashMap::new(),
            surface_layout,
            surface_pipelines,
            full_layout,
            mask_subtract,
            mask_keep,
            dim,
            sampler,
            white,
            black,
            effect_layout,
            effect_pipeline_layout,
            effect_vs,
            effect_pipelines: HashMap::new(),
            effect_targets: HashMap::new(),
            meshes: HashMap::new(),
        };
        r.upload_r8("audio", 128, 3, &[0u8; 384]);
        let (w, h, data) = testgrid::create(1024);
        r.upload_rgba("testgrid", w, h, &data, true);
        r
    }

    // --- Texturspeicher

    pub fn texture(&self, key: &str) -> Option<&TexEntry> {
        self.textures.get(key)
    }

    pub fn has_texture(&self, key: &str) -> bool {
        self.textures.contains_key(key)
    }

    /// RGBA-Bild (vormultipliziert) hochladen; gleiche Größe → Textur wird wiederverwendet
    pub fn upload_rgba(&mut self, key: &str, w: u32, h: u32, data: &[u8], mips: bool) {
        let levels = if mips { mip_count(w, h) } else { 1 };
        let entry = match self.textures.get(key) {
            Some(e) if e.width == w && e.height == h && e.texture.mip_level_count() == levels && e.texture.format() == wgpu::TextureFormat::Rgba8Unorm => {
                e.clone()
            }
            _ => {
                let e = texture_entry(
                    &self.device,
                    key,
                    w,
                    h,
                    wgpu::TextureFormat::Rgba8Unorm,
                    levels,
                    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                );
                self.textures.insert(key.to_string(), e.clone());
                e
            }
        };
        let mut level_data = Cow::Borrowed(data);
        let (mut lw, mut lh) = (w, h);
        for level in 0..levels {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &entry.texture, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &level_data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(lw * 4), rows_per_image: None },
                wgpu::Extent3d { width: lw, height: lh, depth_or_array_layers: 1 },
            );
            if level + 1 < levels {
                let (d, nw, nh) = downsample(&level_data, lw, lh);
                level_data = Cow::Owned(d);
                lw = nw;
                lh = nh;
            }
        }
    }

    /// Einkanaliges Bild (Masken, Audio) hochladen
    pub fn upload_r8(&mut self, key: &str, w: u32, h: u32, data: &[u8]) {
        let entry = match self.textures.get(key) {
            Some(e) if e.width == w && e.height == h => e.clone(),
            _ => {
                let e = texture_entry(&self.device, key, w, h, wgpu::TextureFormat::R8Unorm, 1, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
                self.textures.insert(key.to_string(), e.clone());
                e
            }
        };
        self.queue.write_texture(
            entry.texture.as_image_copy(),
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: None },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }

    pub fn remove_texture(&mut self, key: &str) {
        self.textures.remove(key);
    }

    /// Texturen mit Präfix entfernen, deren Schlüssel nicht mehr gebraucht wird
    pub fn retain_textures(&mut self, prefix: &str, keep: &HashSet<String>) {
        self.textures.retain(|k, _| !k.starts_with(prefix) || keep.contains(k));
    }

    // --- Effekte

    fn effect_pipeline(&mut self, def: &EffectDef) -> Option<wgpu::RenderPipeline> {
        if let Some(p) = self.effect_pipelines.get(&def.id) {
            return p.clone();
        }
        let pipeline = match parse_effect(def) {
            Ok(module) => {
                let fs = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(&def.id),
                    source: wgpu::ShaderSource::Naga(Cow::Owned(module)),
                });
                let format = if def.feedback { FEEDBACK_FORMAT } else { TARGET_FORMAT };
                Some(self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(&def.id),
                    layout: Some(&self.effect_pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &self.effect_vs,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &fs,
                        entry_point: Some("main"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                    }),
                    multiview_mask: None,
                    cache: None,
                }))
            }
            Err(e) => {
                log::error!("Effekt {} fehlerhaft:\n{e}", def.id);
                None
            }
        };
        self.effect_pipelines.insert(def.id.clone(), pipeline.clone());
        pipeline
    }

    /// Rechnet einen Effekt einen Schritt weiter; das Bild liegt danach unter `key` im Texturspeicher.
    pub fn render_effect(&mut self, key: &str, def: &EffectDef, uniforms: &EffectUniforms, w: u32, h: u32) -> bool {
        let Some(pipeline) = self.effect_pipeline(def) else { return false };
        let (w, h) = (w.max(16), h.max(16));
        let feedback = def.feedback;
        let format = if feedback { FEEDBACK_FORMAT } else { TARGET_FORMAT };
        // Neuer Puffer bei anderem Effekt oder deutlich anderer Größe (Rückkopplung behält das Bild sonst)
        let rebuild = match self.effect_targets.get(key) {
            None => true,
            Some(t) => {
                let e = &t.tex[0];
                t.effect != def.id || (e.width as i64 - w as i64).abs() > 32 || (e.height as i64 - h as i64).abs() > 32 || (!feedback && (e.width != w || e.height != h))
            }
        };
        if rebuild {
            let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT;
            let n = if feedback { 2 } else { 1 };
            let tex: Vec<TexEntry> = (0..n).map(|_| texture_entry(&self.device, key, w, h, format, 1, usage)).collect();
            // Rückkopplungspuffer schwarz beginnen
            let mut enc = self.device.create_command_encoder(&Default::default());
            for t in &tex {
                enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &t.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                    })],
                    ..Default::default()
                });
            }
            self.queue.submit([enc.finish()]);
            let uniforms_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("effect-uniforms"),
                size: std::mem::size_of::<EffectUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.effect_targets.insert(key.to_string(), EffectTarget { tex, cur: 0, uniforms: uniforms_buf, effect: def.id.clone() });
        }
        let audio_view = self.textures.get("audio").map(|t| t.view.clone()).unwrap_or_else(|| self.black.view.clone());
        let t = self.effect_targets.get_mut(key).unwrap();
        self.queue.write_buffer(&t.uniforms, 0, bytemuck::bytes_of(uniforms));
        let (src, dst) = if feedback { (t.cur, 1 - t.cur) } else { (0, 0) };
        let prev_view = if feedback { t.tex[src].view.clone() } else { self.black.view.clone() };
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("effect"),
            layout: &self.effect_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: t.uniforms.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&audio_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&prev_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("effect"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.tex[dst].view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([enc.finish()]);
        t.cur = dst;
        let out = t.tex[dst].clone();
        self.textures.insert(key.to_string(), out);
        true
    }

    /// Effekt-Puffer freigeben, die nicht mehr gebraucht werden
    pub fn retain_effects(&mut self, keep: &HashSet<String>) {
        self.effect_targets.retain(|k, _| keep.contains(k));
        self.textures.retain(|k, _| !k.starts_with("fx:") || keep.contains(k));
    }

    pub fn retain_meshes(&mut self, keep: &HashSet<String>) {
        self.meshes.retain(|k, _| keep.contains(k));
    }

    // --- Ansichten

    pub fn create_view_target(&self, width: u32, height: u32) -> ViewTarget {
        let (width, height) = (width.clamp(1, 8192), height.clamp(1, 8192));
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("view"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
        });
        let view = texture.create_view(&Default::default());
        let srgb_view = texture.create_view(&wgpu::TextureViewDescriptor { format: Some(wgpu::TextureFormat::Rgba8UnormSrgb), ..Default::default() });
        let uniform_cap = 64;
        let vertex_cap = 64 * 6;
        ViewTarget {
            texture,
            view,
            srgb_view,
            width,
            height,
            uniforms: self.uniform_buffer(uniform_cap),
            uniform_cap,
            vertices: self.vertex_buffer(vertex_cap),
            vertex_cap,
            egui_id: None,
        }
    }

    fn uniform_buffer(&self, n: u64) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: n * UNIFORM_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn vertex_buffer(&self, n: u64) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vertices"),
            size: n * 16,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn mesh_buffer(&mut self, key: &str, data: &std::sync::Arc<Vec<[f32; 4]>>) -> (wgpu::Buffer, u32) {
        if let Some(m) = self.meshes.get(key) {
            if std::sync::Arc::ptr_eq(&m.data, data) {
                return (m.buffer.clone(), m.count);
            }
        }
        let buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh"),
            contents: bytemuck::cast_slice(data.as_slice()),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let count = data.len() as u32;
        self.meshes.insert(key.to_string(), MeshBuf { data: data.clone(), buffer: buffer.clone(), count });
        (buffer, count)
    }

    /// Zeichnet den Stapel in eine Ansicht.
    /// ref_width: Breite der echten Ausgabe in Pixeln (Rasterlinien skalieren mit);
    /// master: Gesamthelligkeit 0…1; white: Weißbild statt Inhalt.
    pub fn render_view(&mut self, target: &mut ViewTarget, items: &[DrawItem], ref_width: f32, master: f32, white: bool) {
        let px_scale = target.width as f32 / ref_width.max(1.0);
        enum Cmd {
            Surface { pipeline: BlendMode, bind: wgpu::BindGroup, offset: u32, vbuf: Option<wgpu::Buffer>, first: u32, count: u32 },
            Mask { subtract: bool, bind: wgpu::BindGroup },
        }
        let mut uniforms: Vec<SurfaceUniforms> = Vec::new();
        let mut verts: Vec<[f32; 4]> = Vec::new();
        let mut pending: Vec<(usize, Option<(wgpu::Buffer, u32)>, u32, u32, &SurfaceDraw)> = Vec::new();
        let mut masks: Vec<(usize, &str, bool)> = Vec::new();

        if !white && master > 0.0 {
            for (order, item) in items.iter().enumerate() {
                match item {
                    DrawItem::Mask { texture, subtract } => masks.push((order, texture.as_str(), *subtract)),
                    DrawItem::Surface(s) => {
                        let Some(u) = surface_uniforms(s, px_scale) else { continue };
                        let mesh = match (&s.mesh, s.shape) {
                            (Some(m), Shape::Mesh) => Some(self.mesh_buffer(&s.key, m)),
                            _ => None,
                        };
                        let (first, count) = if mesh.is_some() {
                            (0, 0)
                        } else {
                            // Begrenzungsrechteck der Fläche; der Shader verwirft alles außerhalb
                            let pts: &[crate::model::Point] = if s.shape == Shape::Triangle { &s.corners[..3] } else { &s.corners };
                            let x0 = pts.iter().map(|p| p.x).fold(f64::INFINITY, f64::min).max(0.0) as f32;
                            let x1 = pts.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max).min(1.0) as f32;
                            let y0 = pts.iter().map(|p| p.y).fold(f64::INFINITY, f64::min).max(0.0) as f32;
                            let y1 = pts.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max).min(1.0) as f32;
                            if x1 <= x0 || y1 <= y0 {
                                continue;
                            }
                            let first = verts.len() as u32;
                            for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x0, y1), (x1, y0), (x1, y1)] {
                                verts.push([x, y, 0.0, 0.0]);
                            }
                            (first, 6)
                        };
                        let idx = uniforms.len();
                        uniforms.push(u);
                        pending.push((order, mesh, first, count, s));
                        let _ = idx;
                    }
                }
            }
        }

        // Puffer bei Bedarf vergrößern
        if uniforms.len() as u64 > target.uniform_cap {
            target.uniform_cap = (uniforms.len() as u64).next_power_of_two();
            target.uniforms = self.uniform_buffer(target.uniform_cap);
        }
        if verts.len() as u64 > target.vertex_cap {
            target.vertex_cap = (verts.len() as u64).next_power_of_two();
            target.vertices = self.vertex_buffer(target.vertex_cap);
        }
        if !uniforms.is_empty() {
            let mut bytes = vec![0u8; uniforms.len() * UNIFORM_STRIDE as usize];
            for (i, u) in uniforms.iter().enumerate() {
                let o = i * UNIFORM_STRIDE as usize;
                bytes[o..o + std::mem::size_of::<SurfaceUniforms>()].copy_from_slice(bytemuck::bytes_of(u));
            }
            self.queue.write_buffer(&target.uniforms, 0, &bytes);
        }
        if !verts.is_empty() {
            self.queue.write_buffer(&target.vertices, 0, bytemuck::cast_slice(&verts));
        }

        // Zeichenbefehle in Stapelreihenfolge
        let mut cmds: Vec<(usize, Cmd)> = Vec::new();
        for (i, (order, mesh, first, count, s)) in pending.into_iter().enumerate() {
            let tex = match &s.source {
                Source::Texture(k) => match self.textures.get(k) {
                    Some(t) => t.view.clone(),
                    None => continue,
                },
                _ => self.white.view.clone(),
            };
            let mask = match &s.mask {
                Some(k) => match self.textures.get(k) {
                    Some(t) => t.view.clone(),
                    None => self.white.view.clone(),
                },
                None => self.white.view.clone(),
            };
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("surface"),
                layout: &self.surface_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &target.uniforms,
                            offset: 0,
                            size: wgpu::BufferSize::new(std::mem::size_of::<SurfaceUniforms>() as u64),
                        }),
                    },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&tex) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&mask) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ],
            });
            let (vbuf, first, count) = match mesh {
                Some((b, n)) => (Some(b), 0, n),
                None => (None, first, count),
            };
            cmds.push((order, Cmd::Surface { pipeline: s.blend, bind, offset: (i as u64 * UNIFORM_STRIDE) as u32, vbuf, first, count }));
        }
        for (order, key, subtract) in masks {
            let Some(t) = self.textures.get(key) else { continue };
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mask"),
                layout: &self.full_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&t.view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ],
            });
            cmds.push((order, Cmd::Mask { subtract, bind }));
        }
        cmds.sort_by_key(|(o, _)| *o);
        let dim_bind = (master < 1.0 && !white).then(|| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("dim"),
                layout: &self.full_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.white.view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ],
            })
        });

        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let clear = if white { wgpu::Color::WHITE } else { wgpu::Color::BLACK };
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("view"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            for (_, cmd) in &cmds {
                match cmd {
                    Cmd::Surface { pipeline, bind, offset, vbuf, first, count } => {
                        pass.set_pipeline(&self.surface_pipelines[pipeline]);
                        pass.set_bind_group(0, bind, &[*offset]);
                        match vbuf {
                            Some(b) => {
                                pass.set_vertex_buffer(0, b.slice(..));
                                pass.draw(0..*count, 0..1);
                            }
                            None => {
                                pass.set_vertex_buffer(0, target.vertices.slice(..));
                                pass.draw(*first..*first + *count, 0..1);
                            }
                        }
                    }
                    Cmd::Mask { subtract, bind } => {
                        pass.set_pipeline(if *subtract { &self.mask_subtract } else { &self.mask_keep });
                        pass.set_bind_group(0, bind, &[]);
                        pass.draw(0..3, 0..1);
                    }
                }
            }
            if let Some(bind) = &dim_bind {
                let m = master.clamp(0.0, 1.0) as f64;
                pass.set_pipeline(&self.dim);
                pass.set_blend_constant(wgpu::Color { r: m, g: m, b: m, a: m });
                pass.set_bind_group(0, bind, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        self.queue.submit([enc.finish()]);
    }

    /// Ansicht als RGBA-Bild auslesen (für Tests und Bildschirmfotos)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn read_view(&self, target: &ViewTarget) -> Vec<u8> {
        let (w, h) = (target.width, target.height);
        let row = (w * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            target.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let data = slice.get_mapped_range().expect("Lesepuffer");
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let o = (y * row) as usize;
            out.extend_from_slice(&data[o..o + (w * 4) as usize]);
        }
        out
    }
}

fn mat_rows(m: &Mat3) -> [[f32; 4]; 3] {
    [
        [m[0] as f32, m[1] as f32, m[2] as f32, 0.0],
        [m[3] as f32, m[4] as f32, m[5] as f32, 0.0],
        [m[6] as f32, m[7] as f32, m[8] as f32, 0.0],
    ]
}

fn surface_uniforms(s: &SurfaceDraw, px_scale: f32) -> Option<SurfaceUniforms> {
    let is_mesh = s.shape == Shape::Mesh && s.mesh.is_some();
    let rows = if is_mesh {
        [[0.0; 4]; 3]
    } else {
        let inv = if s.shape == Shape::Triangle { invert(&triangle_matrix(&s.corners))? } else { surface_inverse(&s.corners)? };
        mat_rows(&inv)
    };
    let (source, color) = match &s.source {
        Source::Texture(_) => (0.0, [0.0; 4]),
        Source::Color(c) => (1.0, [c[0], c[1], c[2], 1.0]),
        Source::None => (3.0, [0.0; 4]),
    };
    let (grid, grid_color) = match &s.grid {
        Some(g) => ([g.cols, g.rows, g.thickness * px_scale, if g.diagonals { 1.0 } else { 0.0 }], [g.color[0], g.color[1], g.color[2], 1.0]),
        None => ([1.0, 1.0, 1.0, 0.0], [0.0; 4]),
    };
    let shape = match s.shape {
        Shape::Circle => 1.0,
        Shape::Triangle => 2.0,
        _ => 0.0,
    };
    Some(SurfaceUniforms {
        inv0: rows[0],
        inv1: rows[1],
        inv2: rows[2],
        color,
        fit: [s.fit_scale[0], s.fit_scale[1], s.opacity, source],
        grid,
        grid_color,
        flags: [if is_mesh { 1.0 } else { 0.0 }, shape, if s.mask.is_some() { 1.0 } else { 0.0 }, 0.0],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Point;

    #[test]
    fn shaders_validate() {
        for src in [include_str!("surface.wgsl"), include_str!("full.wgsl"), EFFECT_VERT_WGSL] {
            let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
                .validate(&module)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        }
    }

    #[test]
    fn mip_levels() {
        assert_eq!(mip_count(1024, 1024), 11);
        assert_eq!(mip_count(1, 1), 1);
        let (d, w, h) = downsample(&[255u8; 16], 2, 2);
        assert_eq!((w, h, d.len()), (1, 1, 4));
    }

    #[test]
    fn uniforms_for_quad() {
        let s = SurfaceDraw {
            key: "a".into(),
            shape: Shape::Quad,
            corners: [Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(1.0, 1.0), Point::new(0.0, 1.0)],
            mesh: None,
            source: Source::Color([1.0, 0.0, 0.0]),
            opacity: 1.0,
            blend: BlendMode::Normal,
            fit_scale: [1.0, 1.0],
            mask: None,
            grid: None,
        };
        let u = surface_uniforms(&s, 1.0).unwrap();
        // Einheitsquadrat → Einheitsmatrix
        assert!((u.inv0[0] - 1.0).abs() < 1e-6 && u.inv0[1].abs() < 1e-6);
        assert_eq!(u.fit[3], 1.0);
    }
}
