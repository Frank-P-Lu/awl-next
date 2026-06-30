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
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let cache = Cache::new(&device);
        let mut pipeline = TextPipeline::new(&device, &queue, &cache, format);
        pipeline.set_size(width as f32, height as f32);

        Ok(Self {
            instance,
            device,
            queue,
            surface,
            config,
            pipeline,
            window,
        })
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

    pub(super) fn redraw(&mut self) {
        let (w, h) = (self.config.width, self.config.height);
        if let Err(e) = self.pipeline.prepare(&self.device, &self.queue, w, h) {
            eprintln!("prepare error: {e}");
            return;
        }

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                self.window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Suboptimal(_) => {
                self.surface.configure(&self.device, &self.config);
                self.window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.surface = self
                    .instance
                    .create_surface(self.window.clone())
                    .expect("recreate surface");
                self.surface.configure(&self.device, &self.config);
                self.window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                eprintln!("surface validation error");
                return;
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
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
        self.pipeline.atlas.trim();
    }
}
