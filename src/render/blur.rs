//! FROSTED-BACKDROP BLUR — the cached, cheap defocus behind a full-takeover overlay.
//!
//! Today a full overlay (command palette, go-to, outline, keybindings, spell) dimmed
//! the document behind it with a neutral grey scrim, which muted the theme's hues.
//! This replaces that with a real wgpu post-process: when such an overlay opens we
//! render the document ONCE to an offscreen texture, DOWNSAMPLE it to quarter
//! resolution, run a couple of separable-Gaussian ping-pong passes, and composite
//! the frosted result as the backdrop. The blur PRESERVES hue (a defocus, not a
//! desaturation — the whole point); a small dim toward the theme's OWN `base_100`
//! lets the doc recede a value without going neutral.
//!
//! "Do the effect, do it cheap" (PHILOSOPHY): the blur is precomputed + CACHED. The
//! owner ([`super::TextPipeline`]) recomputes only when the captured doc / size /
//! theme actually changes (it tracks a signature); a settled, unchanged
//! overlay-open frame just re-composites the already-blurred quarter texture, so an
//! idle overlay stays 0% CPU (DESIGN §6). It is DETERMINISTIC (no clock) — a pure
//! pixel function of the captured doc — so an overlay capture is byte-stable.
//!
//! EXCEPTIONS (handled by the caller, not here): the THEME PICKER and the
//! CARET-STYLE PICKER stay CRISP (no backdrop at all) — their whole job is showing
//! the live theme colours / caret preview — and the search SPLIT panel keeps the doc
//! bright. So [`super::TextPipeline`] only routes through this module for the
//! blur-eligible full overlays.

use wgpu::util::DeviceExt;

/// Downsample factor: the blur runs at 1/Nth resolution on each axis (N×N fewer
/// pixels), which both speeds the passes and widens the effective blur radius for
/// free. Quarter-res (4) is the sweet spot — clearly frosted, still cheap.
const DOWNSAMPLE: u32 = 4;

/// Number of separable-Gaussian ping-pong ROUNDS (each round = one horizontal + one
/// vertical 9-tap pass). Two rounds on the quarter-res target read as a soft frost
/// without smearing the hues into mud.
const BLUR_ROUNDS: u32 = 2;

/// How far the frosted backdrop dims toward the theme's OWN `base_100` (0 = pure
/// blur, no recede; 1 = the flat base). Small — the doc should still read through the
/// frost, just a value back. Never toward neutral grey (it is the theme's own base).
const DIM: f32 = 0.16;

/// Cap for the doc-capture texture's LARGEST dimension (physical px). The full-res
/// capture is the single biggest transient the blur allocates, yet it only ever feeds
/// the quarter-res downsample + Gaussian — so on a genuinely-large / high-DPI surface
/// (4K/5K) the full resolution is wasted VRAM. Clamping the capture's longest side to
/// this cap sheds that waste with NO visible change (it is blurred + quarter-
/// downsampled either way). Chosen well ABOVE any normal or 2× retina surface, so it
/// only bites when the surface is truly large — every capture at or below the cap is
/// byte-identical.
const DOC_CAPTURE_MAX: u32 = 3200;

/// The doc-capture texture size for a `width`×`height` surface. UNCHANGED at or below
/// [`DOC_CAPTURE_MAX`] (so any normal / retina surface captures full-res and stays
/// byte-identical); above it, scaled DOWN proportionally so the longest side is the
/// cap. Never below the quarter-res blur working size (so the downsample stays a
/// downsample), and never zero. The document is drawn into this texture via the shared
/// glyphon viewport (still sized to the full surface), so a smaller target simply scales
/// the whole document down to fill it — a reduced-scale capture, not a cropped one.
fn capped_doc_size(width: u32, height: u32) -> (u32, u32) {
    let maxd = width.max(height);
    if maxd <= DOC_CAPTURE_MAX || maxd == 0 {
        return (width, height);
    }
    let scale = DOC_CAPTURE_MAX as f32 / maxd as f32;
    let cw = ((width as f32 * scale).round() as u32)
        .max(width / DOWNSAMPLE)
        .max(1);
    let ch = ((height as f32 * scale).round() as u32)
        .max(height / DOWNSAMPLE)
        .max(1);
    (cw, ch)
}

/// Per-pass uniform: the sample step (UV) + the composite tint. MUST match `U` in
/// `shaders/blur.wgsl`.
#[repr(C)]
#[derive(Clone, Copy)]
struct U {
    step: [f32; 4],
    tint: [f32; 4],
}

/// The frosted-backdrop post-process: three fragment pipelines (downsample / blur /
/// composite) over a shared fullscreen-triangle vertex + bind group, plus the
/// lazily-sized offscreen textures they ping-pong through.
pub struct BlurBackdrop {
    down_pipeline: wgpu::RenderPipeline,
    blur_pipeline: wgpu::RenderPipeline,
    comp_pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// The target texture format (the surface / offscreen format the pipelines were
    /// built for); the lazily-created textures must match it.
    format: wgpu::TextureFormat,
    /// The size the current textures + bind groups were built for; `None` until the
    /// first [`Self::ensure`].
    size: Option<(u32, u32)>,
    /// The captured document (full-res render target + sample source).
    doc: Option<wgpu::Texture>,
    doc_view: Option<wgpu::TextureView>,
    /// Quarter-res ping-pong pair.
    qa_view: Option<wgpu::TextureView>,
    qb_view: Option<wgpu::TextureView>,
    /// Per-pass uniform buffers (one value each — a single uniform can't carry
    /// distinct per-pass values within one encoder submit, so each pass owns its own).
    u_down: Option<wgpu::Buffer>,
    u_blur_h: Option<wgpu::Buffer>,
    u_blur_v: Option<wgpu::Buffer>,
    u_comp: Option<wgpu::Buffer>,
    /// Per-source bind groups: down samples the doc, the H passes sample `qa`, the V
    /// passes sample `qb`, the composite samples the final `qa`.
    bg_down: Option<wgpu::BindGroup>,
    bg_h: Option<wgpu::BindGroup>,
    bg_v: Option<wgpu::BindGroup>,
    bg_comp: Option<wgpu::BindGroup>,
}

impl BlurBackdrop {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blur shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/blur.wgsl").into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blur bind layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
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
            label: Some("blur pipeline layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        // All three pipelines share the vertex + layout + (opaque) target format and
        // differ only in the fragment entry point.
        let mk = |entry: &str, label: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        // Each pass overwrites its whole target (a fullscreen tri), so
                        // a plain opaque write — no blending.
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let down_pipeline = mk("fs_down", "blur downsample pipeline");
        let blur_pipeline = mk("fs_blur", "blur gaussian pipeline");
        let comp_pipeline = mk("fs_comp", "blur composite pipeline");
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blur sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        Self {
            down_pipeline,
            blur_pipeline,
            comp_pipeline,
            bind_layout,
            sampler,
            format,
            size: None,
            doc: None,
            doc_view: None,
            qa_view: None,
            qb_view: None,
            u_down: None,
            u_blur_h: None,
            u_blur_v: None,
            u_comp: None,
            bg_down: None,
            bg_h: None,
            bg_v: None,
            bg_comp: None,
        }
    }

    /// (Re)build the textures + bind groups for `width`×`height` and refresh the
    /// per-pass uniforms (sample steps + the composite `tint` toward base_100,
    /// `base100_linear`). Returns `true` when the textures were RECREATED (a fresh /
    /// resized target), so the caller must force a recompute (the cached blur is
    /// gone). A same-size call only re-uploads the uniforms and returns `false`.
    pub fn ensure(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        base100_linear: [f32; 3],
    ) -> bool {
        let recreated = self.size != Some((width, height));
        if recreated {
            // DROP-BEFORE-ALLOCATE: release the PREVIOUS textures/views/bind-groups
            // (set them to `None`) BEFORE creating the new ones, so a resize never has
            // the old AND new doc/qa/qb sets live at the same instant — that transient
            // double is the resize VRAM peak. The size-guard above means we only reach
            // here on a GENUINE size change, so the final resources are identical to the
            // un-dropped path; only the momentary doubling is gone.
            self.bg_down = None;
            self.bg_h = None;
            self.bg_v = None;
            self.bg_comp = None;
            self.doc = None;
            self.doc_view = None;
            self.qa_view = None;
            self.qb_view = None;
            self.u_down = None;
            self.u_blur_h = None;
            self.u_blur_v = None;
            self.u_comp = None;

            let format = self.format;
            let qw = (width / DOWNSAMPLE).max(1);
            let qh = (height / DOWNSAMPLE).max(1);
            // Cap the full-res doc capture on very-large/high-DPI surfaces (no-op at or
            // below the cap → byte-identical); a smaller target scales the whole document
            // down to fill it (see `capped_doc_size`).
            let (cw, ch) = capped_doc_size(width, height);
            let mk_tex = |label: &str, w: u32, h: u32| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
            };
            let doc = mk_tex("blur doc", cw, ch);
            let qa = mk_tex("blur qa", qw, qh);
            let qb = mk_tex("blur qb", qw, qh);
            let v = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
            let doc_view = v(&doc);
            let qa_view = v(&qa);
            let qb_view = v(&qb);

            // Sample steps: the downsample reads the full-res doc; each blur axis
            // reads the quarter-res texel along one direction.
            let dummy_tint = [0.0, 0.0, 0.0, 0.0];
            // The downsample's 4-tap box reads ONE texel of the (possibly capped) doc,
            // so its step is the capped doc's texel size, not the surface's.
            let u_down = self.mk_uniform(device, [1.0 / cw as f32, 1.0 / ch as f32, 0.0, 0.0], dummy_tint);
            let u_blur_h = self.mk_uniform(device, [1.0 / qw as f32, 0.0, 0.0, 0.0], dummy_tint);
            let u_blur_v = self.mk_uniform(device, [0.0, 1.0 / qh as f32, 0.0, 0.0], dummy_tint);
            let u_comp = self.mk_uniform(device, [0.0; 4], [base100_linear[0], base100_linear[1], base100_linear[2], DIM]);

            self.bg_down = Some(self.mk_bind(device, &u_down, &doc_view));
            self.bg_h = Some(self.mk_bind(device, &u_blur_h, &qa_view));
            self.bg_v = Some(self.mk_bind(device, &u_blur_v, &qb_view));
            self.bg_comp = Some(self.mk_bind(device, &u_comp, &qa_view));

            self.doc = Some(doc);
            self.doc_view = Some(doc_view);
            self.qa_view = Some(qa_view);
            self.qb_view = Some(qb_view);
            self.u_down = Some(u_down);
            self.u_blur_h = Some(u_blur_h);
            self.u_blur_v = Some(u_blur_v);
            self.u_comp = Some(u_comp);
            self.size = Some((width, height));
        }
        // Refresh the composite tint each call (cheap) so a theme change between
        // captures lands the right base_100 without a texture rebuild.
        if let Some(buf) = &self.u_comp {
            let u = U {
                step: [0.0; 4],
                tint: [base100_linear[0], base100_linear[1], base100_linear[2], DIM],
            };
            queue.write_buffer(buf, 0, bytes_of(&u));
        }
        recreated
    }

    fn mk_uniform(&self, device: &wgpu::Device, step: [f32; 4], tint: [f32; 4]) -> wgpu::Buffer {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("blur uniform"),
            contents: bytes_of(&U { step, tint }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    fn mk_bind(
        &self,
        device: &wgpu::Device,
        uniform: &wgpu::Buffer,
        view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blur bind"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// The full-res render target the caller draws the document into (so it can be
    /// captured, then blurred). `None` before the first [`Self::ensure`].
    pub fn doc_view(&self) -> Option<&wgpu::TextureView> {
        self.doc_view.as_ref()
    }

    /// Run the downsample + the separable-Gaussian ping-pong passes into the
    /// quarter-res pair, leaving the FINAL blurred result in `qa` (which
    /// [`Self::draw_backdrop`] composites). Each pass is its own render pass on the
    /// shared encoder, so wgpu inserts the read-after-write barriers between them.
    /// The doc texture must already be drawn (an earlier pass on `doc_view`).
    pub fn encode_blur(&self, encoder: &mut wgpu::CommandEncoder) {
        let (Some(qa), Some(qb), Some(bg_down), Some(bg_h), Some(bg_v)) = (
            &self.qa_view,
            &self.qb_view,
            &self.bg_down,
            &self.bg_h,
            &self.bg_v,
        ) else {
            return;
        };
        // 1) downsample doc -> qa.
        self.pass(encoder, &self.down_pipeline, bg_down, qa);
        // 2) BLUR_ROUNDS of (H: qa -> qb, V: qb -> qa). bg_h samples qa, bg_v samples
        //    qb, so the same two bind groups serve every round.
        for _ in 0..BLUR_ROUNDS {
            self.pass(encoder, &self.blur_pipeline, bg_h, qb);
            self.pass(encoder, &self.blur_pipeline, bg_v, qa);
        }
    }

    /// One fullscreen-triangle pass: clear the target (the tri overwrites every
    /// pixel, so the clear is just a defined load) and draw with `pipeline` + `bind`.
    fn pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        bind: &wgpu::BindGroup,
        target: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("blur pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Composite the cached frosted backdrop (the final blurred `qa`, upsampled +
    /// dimmed toward base_100) into an already-open render pass — drawn FIRST in the
    /// final pass, before the overlay card/text. A no-op until [`Self::ensure`] has run.
    pub fn draw_backdrop<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if let Some(bg) = &self.bg_comp {
            pass.set_pipeline(&self.comp_pipeline);
            pass.set_bind_group(0, bg, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

/// Reinterpret a `#[repr(C)]` POD as bytes for an upload (same minimal shim the
/// other pipelines use; the type is a small f32 array struct with no padding).
fn bytes_of(u: &U) -> &[u8] {
    unsafe {
        core::slice::from_raw_parts((u as *const U) as *const u8, core::mem::size_of::<U>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_capture_cap_is_a_noop_at_or_below_the_cap() {
        // A normal surface, a 2× retina surface, and exactly the cap all pass through
        // UNCHANGED — so the capture (and thus the blurred backdrop) is byte-identical.
        assert_eq!(capped_doc_size(1200, 800), (1200, 800));
        assert_eq!(capped_doc_size(2400, 1600), (2400, 1600));
        assert_eq!(capped_doc_size(DOC_CAPTURE_MAX, 1000), (DOC_CAPTURE_MAX, 1000));
        assert_eq!(capped_doc_size(1000, DOC_CAPTURE_MAX), (1000, DOC_CAPTURE_MAX));
    }

    #[test]
    fn doc_capture_cap_scales_a_genuinely_large_surface_and_preserves_aspect() {
        // A 5K surface: the longest side is clamped to the cap, the short side scaled
        // by the same factor (aspect preserved), and the result stays at least the
        // quarter-res blur working size so the downsample is still a downsample.
        let (cw, ch) = capped_doc_size(5120, 2880);
        assert_eq!(cw, DOC_CAPTURE_MAX);
        let scale = DOC_CAPTURE_MAX as f32 / 5120.0;
        assert_eq!(ch, (2880.0 * scale).round() as u32);
        assert!(cw >= 5120 / DOWNSAMPLE && ch >= 2880 / DOWNSAMPLE);
        // Portrait orientation clamps on height instead.
        let (pw, ph) = capped_doc_size(2880, 5120);
        assert_eq!(ph, DOC_CAPTURE_MAX);
        assert_eq!(pw, (2880.0 * scale).round() as u32);
    }
}
