//! GPU surface + frame loop: the wgpu device/queue/surface, the swap-chain
//! reconfigure, and the per-frame prepare/render of the shared [`TextPipeline`].
//! Carved out of `app.rs` verbatim; the methods stay inherent on [`super::Gpu`]
//! (a child module sees its parent's private `Gpu` fields), so behaviour — and
//! the capture output — is byte-identical.

use super::*;

impl Gpu {
    // Takes the display handle BY VALUE (not `&ActiveEventLoop`) so the wasm path
    // can move it into a `'static` `spawn_local` future — async GPU init can't
    // borrow the event loop across the await. Native passes
    // `event_loop.owned_display_handle()` at the call site, unchanged in effect.
    pub(super) async fn new(
        window: Arc<Window>,
        display_handle: winit::event_loop::OwnedDisplayHandle,
    ) -> anyhow::Result<Self> {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(display_handle),
        ));

        let surface = instance.create_surface(window.clone())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|e| anyhow::anyhow!("no adapter: {e:?}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl device"),
                ..Default::default()
            })
            .await?;

        let caps = surface.get_capabilities(&adapter);
        // The CONFIG format: prefer a platform-offered sRGB surface format (native
        // Metal/Vulkan list `Bgra8UnormSrgb`); else the first advertised format. On
        // the WebGPU/WebGL2 canvas the caps list ONLY non-srgb formats
        // (`bgra8unorm`/`rgba8unorm` — the WebGPU spec forbids an `*-srgb` primary
        // canvas format), so this lands on a NON-srgb `config_format` there.
        let config_format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        // The VIEW format we actually render through — always the sRGB variant, so
        // the hardware applies the linear→sRGB encode on write (the shaders + the
        // per-pipeline color converters all emit LINEAR light expecting exactly
        // that). On native `config_format` is already srgb, so this is a no-op
        // (`view_format == config_format`, `view_formats` stays empty → the surface
        // config is byte-identical to before). On the web it upgrades the non-srgb
        // canvas: we list the srgb variant in `view_formats` and create the frame
        // view with it in `redraw`, which is the WebGPU-blessed way to get an sRGB
        // canvas (config a base format, render through an srgb view). WITHOUT this
        // the web surface stores the linearised grounds raw and the scene reads far
        // too dark (Tawny's margins collapse from (27,29,35) to near-black (3,3,4)).
        let view_format = config_format.add_srgb_suffix();
        let view_formats = if view_format != config_format {
            vec![view_format]
        } else {
            vec![]
        };

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: config_format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats,
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let cache = Cache::new(&device);
        // The whole pipeline (glyphon + every quad pipeline) targets the srgb VIEW
        // format, never the possibly-non-srgb config format.
        let mut pipeline = TextPipeline::new(&device, &queue, &cache, view_format);
        pipeline.set_size(width as f32, height as f32);

        Ok(Self {
            instance,
            device,
            queue,
            surface,
            config,
            view_format,
            pipeline,
            window,
        })
    }

    /// The GPU's CURRENT allocated memory in BYTES for the debug panel's `gpu N MB`
    /// line, or `None` when there is no cheap query. macOS reads Metal's
    /// `MTLDevice.currentAllocatedSize` straight off the raw device through wgpu-hal;
    /// Vulkan-without-ext and WebGPU have no equivalent, so they (and any non-macOS
    /// target) return `None` and the panel shows `gpu —`. Live-only — device state, never
    /// part of a deterministic capture.
    #[cfg(target_os = "macos")]
    pub(super) fn current_gpu_bytes(&self) -> Option<u64> {
        use objc2_metal::MTLDevice;
        // SAFETY: `as_hal` hands back a borrow of the live Metal device; we only READ
        // `currentAllocatedSize` off it and drop the guard immediately — no resource is
        // retained, destroyed, or used past the closure.
        unsafe {
            self.device
                .as_hal::<wgpu::hal::api::Metal>()
                .map(|d| d.raw_device().currentAllocatedSize() as u64)
        }
    }

    /// Non-macOS: no cheap GPU-memory query (Vulkan-without-ext / WebGPU), so the debug
    /// panel shows the `gpu —` placeholder.
    #[cfg(not(target_os = "macos"))]
    pub(super) fn current_gpu_bytes(&self) -> Option<u64> {
        None
    }

    pub(super) fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.pipeline.set_size(width as f32, height as f32);
    }

    /// Draw one frame. Returns `Some((cost_ms, present_return))` when the frame
    /// actually PRESENTED and the debug panel is on — the frame's CPU COST in ms
    /// plus the instant `frame.present()` returned (the key→px latency endpoint)
    /// — and `None` on every early-return path (surface retry / validation) or
    /// while the panel is off (zero timing work, per the pane's own contract).
    ///
    /// The COST is the busy/wait split done honestly: (prepare duration) +
    /// (post-acquire encode + submit + present-return), EXPLICITLY EXCLUDING the
    /// `get_current_texture` acquire block — under Fifo back-pressure the acquire
    /// wait is vsync PACING, not work, and folding it in would make every busy
    /// sequence read as exactly-at-budget (PresentMon's MsCPUBusy vs MsCPUWait
    /// distinction). Stamps use the wasm-safe `crate::clock::Instant`.
    pub(super) fn redraw(&mut self) -> Option<(f32, Instant)> {
        let (w, h) = (self.config.width, self.config.height);
        let debug = crate::debug::debug_on();
        let t0 = debug.then(Instant::now);
        if let Err(e) = self.pipeline.prepare(&self.device, &self.queue, w, h) {
            eprintln!("prepare error: {e}");
            return None;
        }
        // Prepare's span ends here; the acquire wait below is excluded.
        let prepare_ms = t0.map(|t| t.elapsed().as_secs_f32() * 1000.0);

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                self.window.request_redraw();
                return None;
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Suboptimal(_) => {
                self.surface.configure(&self.device, &self.config);
                self.window.request_redraw();
                return None;
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.surface = self
                    .instance
                    .create_surface(self.window.clone())
                    .expect("recreate surface");
                self.surface.configure(&self.device, &self.config);
                self.window.request_redraw();
                return None;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                eprintln!("surface validation error");
                return None;
            }
        };
        // Acquire SUCCEEDED: the post-acquire span (encode + submit + present).
        let t2 = debug.then(Instant::now);

        // Render through the sRGB VIEW format (see `Gpu::new`): on native this is
        // the config format itself (a no-op reinterpretation); on the web it is the
        // srgb variant listed in `config.view_formats`, so the frame gets the
        // linear→sRGB encode on write.
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.view_format),
            ..Default::default()
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("awl frame encoder"),
            });
        if let Err(e) = self.pipeline.render(&mut encoder, &view) {
            eprintln!("render error: {e}");
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
        // The latency endpoint: present-SUBMISSION return (wgpu exposes no
        // presented-time), stamped before the off-frame atlas trim.
        let done = debug.then(Instant::now);
        self.pipeline.atlas.trim();
        match (prepare_ms, t2, done) {
            (Some(prep), Some(t2), Some(done)) => {
                Some((prep + (done - t2).as_secs_f32() * 1000.0, done))
            }
            _ => None,
        }
    }
}
