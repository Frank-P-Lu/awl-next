//! GPU surface + frame loop: the wgpu device/queue/surface, the swap-chain
//! reconfigure, and the per-frame prepare/render of the shared [`TextPipeline`].
//! Carved out of `app.rs` verbatim; the methods stay inherent on [`super::Gpu`]
//! (a child module sees its parent's private `Gpu` fields), so behaviour — and
//! the capture output — is byte-identical.

use super::*;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GpuFaultKind {
    OutOfMemory,
    Validation,
    Internal,
    DeviceLost,
    SurfaceRecoveryFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg(not(target_arch = "wasm32"))]
pub(super) enum GpuFaultInjection { OutOfMemory, DeviceLost }

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct GpuFault { pub(super) kind: GpuFaultKind, pub(super) message: String }

#[derive(Default)]
struct FaultSlots {
    device_lost: Option<String>, out_of_memory: Option<String>, surface: Option<String>,
    internal: Option<String>, validation: Option<String>,
}

impl FaultSlots {
    fn drain(&mut self) -> Vec<GpuFault> {
        [
            (GpuFaultKind::DeviceLost, self.device_lost.take()),
            (GpuFaultKind::SurfaceRecoveryFailed, self.surface.take()),
            (GpuFaultKind::Internal, self.internal.take()),
            (GpuFaultKind::OutOfMemory, self.out_of_memory.take()),
            (GpuFaultKind::Validation, self.validation.take()),
        ].into_iter().filter_map(|(kind, message)| message.map(|message| GpuFault { kind, message })).collect()
    }
}

#[derive(Clone)]
pub(super) struct GpuFaultInbox { slots: Arc<Mutex<FaultSlots>>, window: Arc<Window> }

impl GpuFaultInbox {
    fn new(window: Arc<Window>) -> Self {
        Self { slots: Arc::new(Mutex::new(FaultSlots::default())), window }
    }
    fn report(&self, fault: GpuFault) {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        let slot = match fault.kind {
            GpuFaultKind::DeviceLost => &mut slots.device_lost,
            GpuFaultKind::OutOfMemory => &mut slots.out_of_memory,
            GpuFaultKind::SurfaceRecoveryFailed => &mut slots.surface,
            GpuFaultKind::Internal => &mut slots.internal,
            GpuFaultKind::Validation => &mut slots.validation,
        };
        *slot = Some(fault.message);
        drop(slots);
        self.window.request_redraw();
    }
    fn drain(&self) -> Vec<GpuFault> {
        let mut s = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        s.drain()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GpuFrameSkip { Timeout, Occluded, SurfaceReconfigured, SurfaceRecreated, PrepareFailed }
pub(super) enum GpuFrameOutcome { Presented(Option<(f32, Instant)>), Skipped(GpuFrameSkip), Fault(GpuFault) }
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GpuResizeOutcome { IgnoredZeroExtent, Reconfigured }

fn classify_uncaptured(error: wgpu::Error) -> GpuFault {
    let kind = match error {
        wgpu::Error::OutOfMemory { .. } => GpuFaultKind::OutOfMemory,
        wgpu::Error::Validation { .. } => GpuFaultKind::Validation,
        wgpu::Error::Internal { .. } => GpuFaultKind::Internal,
    };
    GpuFault { kind, message: error.to_string() }
}

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
        #[cfg(not(target_arch = "wasm32"))]
        let backend_name = {
            let info = adapter.get_info();
            format!("{:?}: {}", info.backend, info.name)
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("awl device"),
                required_limits: Self::device_limits(&adapter),
                ..Default::default()
            })
            .await?;

        let faults = GpuFaultInbox::new(window.clone());
        let uncaptured = faults.clone();
        device.on_uncaptured_error(Arc::new(move |error| uncaptured.report(classify_uncaptured(error))));
        let lost = faults.clone();
        device.set_device_lost_callback(move |reason, message| {
            lost.report(GpuFault { kind: GpuFaultKind::DeviceLost, message: format!("{reason:?}: {message}") });
        });

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
            .or_else(|| caps.formats.first().copied())
            .ok_or_else(|| anyhow::anyhow!("surface advertised no texture formats"))?;
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

        // LIVE PROBE (`--live-script`): the surface additionally allows COPY_SRC
        // so every presented frame can be blitted into the probe's mirror
        // texture (`mirror_presented_frame`). Metal supports it; every normal
        // launch keeps the production usage bit-for-bit.
        #[allow(unused_mut)]
        let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        #[cfg(not(target_arch = "wasm32"))]
        if crate::probe::live_active() {
            usage |= wgpu::TextureUsages::COPY_SRC;
        }
        let config = wgpu::SurfaceConfiguration {
            usage,
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
        // MOTION-JUICE ARMING — the ONE call site, on the live App's GPU init
        // alone: every headless capture / bench / test pipeline stays unarmed,
        // so those paths render the settled state structurally (the
        // determinism law). Arming is inert on its own — every world ships
        // `MotionJuice::CALM`; see `TextPipeline::arm_live_juice`'s doc.
        pipeline.arm_live_juice();

        Ok(Self {
            instance,
            device,
            queue,
            surface,
            config,
            view_format,
            pipeline,
            window,
            #[cfg(not(target_arch = "wasm32"))]
            backend_name,
            faults,
            #[cfg(not(target_arch = "wasm32"))]
            inject_surface_loss: false,
            #[cfg(not(target_arch = "wasm32"))]
            probe_mirror: None,
        })
    }

    /// The LIMITS requested from the device. Native keeps wgpu's own bare
    /// `Limits::default()` (the full, unconstrained tier — Metal/Vulkan
    /// comfortably exceed it; unchanged from before this fix, so native init +
    /// every capture stays byte-identical).
    ///
    /// On wasm the adapter may be a WebGL2 fallback (a browser with no WebGPU
    /// support) rather than a real WebGPU one: `Limits::default()` demands
    /// COMPUTE-shader limits (e.g. `max_compute_workgroups_per_dimension`) that
    /// a WebGL2 adapter reports as 0, so `request_device` rejected the request
    /// outright and the canvas never painted — a blank page on any no-WebGPU
    /// browser (`gallery/web-webgl2-initfail.png`, the color-fix round's
    /// evidence). `Limits::downlevel_webgl2_defaults()` is wgpu's own
    /// WebGL2-safe floor; `.using_resolution(adapter.limits())` then raises
    /// every limit back up to whatever THIS adapter actually reports — so a
    /// real WebGPU adapter (Chrome/Edge with WebGPU on) still gets its full
    /// limits, never clamped down to the WebGL2 floor, while a WebGL2-only
    /// adapter (Safari, or WebGPU-off) gets a request it can satisfy instead of
    /// an instant failure. The canonical shape for this exact failure (see the
    /// `wgpu` examples' own web setup).
    #[cfg(target_arch = "wasm32")]
    fn device_limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
        wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn device_limits(_adapter: &wgpu::Adapter) -> wgpu::Limits {
        wgpu::Limits::default()
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

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn backend_name(&self) -> &str { &self.backend_name }

    pub(super) fn resize(&mut self, width: u32, height: u32) -> GpuResizeOutcome {
        if width == 0 || height == 0 {
            return GpuResizeOutcome::IgnoredZeroExtent;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.pipeline.set_size(width as f32, height as f32);
        GpuResizeOutcome::Reconfigured
    }

    pub(super) fn take_faults(&self) -> Vec<GpuFault> { self.faults.drain() }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn inject_fault(&self, injection: GpuFaultInjection) {
        let kind = match injection { GpuFaultInjection::OutOfMemory => GpuFaultKind::OutOfMemory, GpuFaultInjection::DeviceLost => GpuFaultKind::DeviceLost };
        self.faults.report(GpuFault { kind, message: format!("injected {kind:?}") });
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn inject_surface_loss(&mut self) { self.inject_surface_loss = true; self.window.request_redraw(); }

    fn recover_surface(&mut self) -> Result<(), GpuFault> {
        let surface = self.instance.create_surface(self.window.clone()).map_err(|e| GpuFault {
            kind: GpuFaultKind::SurfaceRecoveryFailed, message: format!("could not recreate GPU surface: {e}"),
        })?;
        surface.configure(&self.device, &self.config);
        self.surface = surface;
        Ok(())
    }

    /// LIVE-RESIZE CONTENT-STRETCH FIX (macOS only): toggle the underlying
    /// `CAMetalLayer`'s own `presentsWithTransaction` flag.
    ///
    /// Metal's `CAMetalLayer` presents ASYNCHRONOUSLY by default — a `present()`
    /// is handed to the compositor independently of AppKit's own window-resize
    /// animation. During a FAST live-resize drag, AppKit commits a new
    /// Core Animation transaction for the window's growing/shrinking bounds on
    /// every drag tick; if OUR next frame hasn't presented yet when that
    /// transaction commits, the compositor has nothing fresh at the new size to
    /// show and instead SCALES the last-presented (stale-size) drawable to cover
    /// the new bounds — the classic macOS "content stretches while you drag
    /// fast" artifact (confirmed as the user's actual live symptom: a slow,
    /// one-pixel-at-a-time drag was already smooth after the earlier rail-entry-
    /// ramp fix; only a FAST drag still visibly stretched — a compositor-level
    /// effect the headless harness cannot see, since it never opens a real
    /// window). Setting `presentsWithTransaction(true)` makes our `present()`
    /// JOIN that same transaction instead of racing it, so the window waits for
    /// our frame before committing the resize — content stays crisp at every
    /// drag speed.
    ///
    /// **The documented trade-off** (Apple's own note on the property): a
    /// transaction-synced present costs a little throughput / can make the
    /// drag feel a touch "heavier" — so this is armed ON only while `Resized`
    /// events are actively arriving (`App::arm_live_resize_sync`) and flipped
    /// back OFF a short settle period after the last one (`App::about_to_wait`'s
    /// `RESIZE_SYNC_SETTLE` debounce), rather than left on permanently.
    #[cfg(target_os = "macos")]
    pub(super) fn set_presents_with_transaction(&self, on: bool) {
        // SAFETY: this only reads the Metal surface handle to flip one plain
        // boolean property on its `CAMetalLayer`, then drops the guard
        // immediately — no resource is retained, destroyed, or used past this
        // call (mirrors `current_gpu_bytes`'s identical `as_hal` shape above).
        unsafe {
            if let Some(surf) = self.surface.as_hal::<wgpu::hal::api::Metal>() {
                surf.render_layer().lock().setPresentsWithTransaction(on);
            }
        }
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
    /// LIVE PROBE only: encode a copy of the just-rendered surface texture
    /// into the persistent mirror (lazily (re)created on size/format change,
    /// e.g. a resize mid-script). Runs inside `redraw`'s own encoder so the
    /// mirror can never hold a HALF-newer frame than what presents.
    #[cfg(not(target_arch = "wasm32"))]
    fn mirror_presented_frame(&mut self, encoder: &mut wgpu::CommandEncoder, frame: &wgpu::Texture) {
        let (w, h) = (frame.width(), frame.height());
        let stale = self
            .probe_mirror
            .as_ref()
            .is_none_or(|m| m.width() != w || m.height() != h || m.format() != frame.format());
        if stale {
            self.probe_mirror = Some(self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("awl live-probe frame mirror"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: frame.format(),
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            }));
        }
        let mirror = self.probe_mirror.as_ref().expect("mirror just ensured");
        encoder.copy_texture_to_texture(
            frame.as_image_copy(),
            mirror.as_image_copy(),
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }

    /// LIVE PROBE only: read the mirror — the LAST PRESENTED frame — back as a
    /// tight RGBA image (sRGB bytes, PNG-ready; BGRA surfaces channel-swapped).
    /// `Err` names the reason (no frame presented yet / readback failure) for
    /// the probe's stdout protocol line.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn read_probe_mirror(&self) -> Result<image::RgbaImage, String> {
        let mirror = self
            .probe_mirror
            .as_ref()
            .ok_or_else(|| "no frame has presented yet".to_string())?;
        let (w, h) = (mirror.width(), mirror.height());
        let mut img = crate::capture::gpu::read_frame(&self.device, &self.queue, mirror, w, h)
            .map_err(|e| format!("mirror readback failed: {e}"))?;
        if matches!(
            mirror.format(),
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        ) {
            for px in img.pixels_mut() {
                px.0.swap(0, 2);
            }
        }
        Ok(img)
    }

    pub(super) fn redraw(&mut self) -> GpuFrameOutcome {
        let (w, h) = (self.config.width, self.config.height);
        let debug = crate::debug::debug_on();
        let t0 = debug.then(Instant::now);
        if let Err(e) = self.pipeline.prepare(&self.device, &self.queue, w, h) {
            eprintln!("prepare error: {e}");
            return GpuFrameOutcome::Skipped(GpuFrameSkip::PrepareFailed);
        }
        if let Some(fault) = self.take_faults().into_iter().next() { return GpuFrameOutcome::Fault(fault); }
        // Prepare's span ends here; the acquire wait below is excluded.
        let prepare_ms = t0.map(|t| t.elapsed().as_secs_f32() * 1000.0);

        #[cfg(not(target_arch = "wasm32"))]
        let acquired = if std::mem::take(&mut self.inject_surface_loss) { wgpu::CurrentSurfaceTexture::Lost } else { self.surface.get_current_texture() };
        #[cfg(target_arch = "wasm32")]
        let acquired = self.surface.get_current_texture();
        let frame = match acquired {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return GpuFrameOutcome::Skipped(if matches!(acquired, wgpu::CurrentSurfaceTexture::Timeout) { GpuFrameSkip::Timeout } else { GpuFrameSkip::Occluded });
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Suboptimal(_) => {
                self.surface.configure(&self.device, &self.config);
                return GpuFrameOutcome::Skipped(GpuFrameSkip::SurfaceReconfigured);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                if let Err(fault) = self.recover_surface() { return GpuFrameOutcome::Fault(fault); }
                return GpuFrameOutcome::Skipped(GpuFrameSkip::SurfaceRecreated);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return GpuFrameOutcome::Fault(GpuFault { kind: GpuFaultKind::Validation, message: "surface validation error".into() });
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
        // LIVE PROBE frame mirror: blit the finished frame into the persistent
        // mirror texture INSIDE the same submission, so the mirror always holds
        // exactly the last frame the compositor was handed. A no-op branch on
        // every normal launch.
        #[cfg(not(target_arch = "wasm32"))]
        if crate::probe::live_active() {
            self.mirror_presented_frame(&mut encoder, &frame.texture);
        }
        self.queue.submit(Some(encoder.finish()));
        // Notify winit we're about to present, per its own documented practice
        // ("call this after drawing, before you submit the buffer to the
        // display"). A no-op on macOS/X11/Windows/Web (winit lists it
        // "Unsupported" there) — the platform this matters for is Wayland,
        // where it schedules winit's own frame-callback throttling for
        // `RedrawRequested`; harmless everywhere else, so unconditional.
        self.window.pre_present_notify();
        frame.present();
        #[cfg(not(target_arch = "wasm32"))]
        if crate::probe::live_active() {
            crate::probe::trace(format_args!("present"));
        }
        // The latency endpoint: present-SUBMISSION return (wgpu exposes no
        // presented-time), stamped before the off-frame atlas trim.
        let done = debug.then(Instant::now);
        self.pipeline.atlas.trim();
        match (prepare_ms, t2, done) {
            (Some(prep), Some(t2), Some(done)) => {
                GpuFrameOutcome::Presented(Some((prep + (done - t2).as_secs_f32() * 1000.0, done)))
            }
            _ => GpuFrameOutcome::Presented(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    fn source() -> wgpu::ErrorSource { Box::new(io::Error::other("synthetic")) }
    #[test] fn uncaptured_oom_is_not_validation() { assert_eq!(classify_uncaptured(wgpu::Error::OutOfMemory { source: source() }).kind, GpuFaultKind::OutOfMemory); }
    #[test] fn uncaptured_validation_stays_distinct_from_oom() { assert_eq!(classify_uncaptured(wgpu::Error::Validation { source: source(), description: "bad".into() }).kind, GpuFaultKind::Validation); }
    #[test] fn uncaptured_internal_stays_distinct_from_device_loss() { assert_eq!(classify_uncaptured(wgpu::Error::Internal { source: source(), description: "bad".into() }).kind, GpuFaultKind::Internal); }
    #[test]
    fn bounded_fault_slots_prioritize_rebuild_before_retry() {
        let mut slots = FaultSlots::default();
        for n in 0..1_000 {
            slots.out_of_memory = Some(format!("oom {n}"));
        }
        slots.internal = Some("internal".into());
        slots.validation = Some("validation".into());
        let faults = slots.drain();
        assert_eq!(faults.len(), 3);
        assert_eq!(faults[0].kind, GpuFaultKind::Internal);
        assert_eq!(faults[1].kind, GpuFaultKind::OutOfMemory);
        assert_eq!(faults[1].message, "oom 999");
        assert_eq!(faults[2].kind, GpuFaultKind::Validation);
    }
}
