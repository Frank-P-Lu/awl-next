//! Headless wgpu plumbing shared by every capture variant: the device/queue
//! request, the offscreen color target, and the row-aligned texture readback (plus
//! the 256-byte alignment helper they all lean on). Carved out of `capture.rs`
//! VERBATIM — the boilerplate is identical for the single-frame and both per-step
//! loops, so it lives here once. See [`super`].

use anyhow::{Context, Result};

use super::FORMAT;

/// Round a row byte count up to wgpu's required 256-byte alignment for buffer
/// copies (`COPY_BYTES_PER_ROW_ALIGNMENT`).
fn align_256(n: u32) -> u32 {
    (n + 255) & !255
}

/// Request a headless wgpu device + queue — no surface, no window. The adapter /
/// device boilerplate is identical for every capture variant, so it lives here
/// once; all three async entry points open on this.
pub(super) async fn headless_device() -> Result<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    // (capture runs without a window, so no display handle is needed)
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .context("no wgpu adapter for headless capture")?;
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("awl headless device"),
            ..Default::default()
        })
        .await
        .context("request_device failed")
}

/// Create the offscreen color target (texture + its default view) for a headless
/// render: a single-sample [`FORMAT`] texture usable as a render attachment AND a
/// copy source. The descriptor is the same in every variant, so it lives here once.
pub(super) fn offscreen_target(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("awl offscreen"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Read an already-rendered offscreen `texture` back to the CPU as a tight
/// [`image::RgbaImage`]: allocate a row-aligned readback buffer, encode + submit the
/// texture->buffer copy, map + poll to completion, then drop wgpu's 256-byte row
/// padding into a packed RGBA image. The caret/document must already be drawn into
/// `texture` (submitted) before calling. Shared by the single-frame path and BOTH
/// per-step loops so the readback dance lives in one place.
pub(super) fn read_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Result<image::RgbaImage> {
    // --- Readback buffer (row-aligned) -----------------------------------
    let unpadded_bpr = width * 4; // RGBA8
    let padded_bpr = align_256(unpadded_bpr);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("awl readback"),
        size: (padded_bpr * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    // --- Encode + submit the texture -> buffer copy ----------------------
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("awl capture copy encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bpr),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    // --- Map and read back -----------------------------------------------
    let (tx, rx) = std::sync::mpsc::channel();
    readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .context("device poll failed")?;
    rx.recv()
        .context("map_async channel closed")?
        .context("buffer map failed")?;

    // Drop padding: copy each row's unpadded prefix into a tight RGBA buffer.
    let mut rgba = vec![0u8; (unpadded_bpr * height) as usize];
    {
        let mapped = readback.slice(..).get_mapped_range();
        for y in 0..height {
            let src = (y * padded_bpr) as usize;
            let dst = (y * unpadded_bpr) as usize;
            rgba[dst..dst + unpadded_bpr as usize]
                .copy_from_slice(&mapped[src..src + unpadded_bpr as usize]);
        }
    }
    readback.unmap();

    image::RgbaImage::from_raw(width, height, rgba)
        .context("failed to build RgbaImage from readback")
}
