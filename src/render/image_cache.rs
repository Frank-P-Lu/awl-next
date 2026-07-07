//! INLINE IMAGES — the decode + GPU-upload cache.
//!
//! Keyed by the image's CANONICAL path (a stable global identity, so the cache
//! survives buffer swaps — the cache-key discipline: a per-buffer version would
//! collide across opens) with the file's MTIME stored alongside, so an image
//! edited on disk re-decodes on its next visible frame. Each entry is either
//! [`ImageState::Ready`] (a decoded `Rgba8UnormSrgb` texture + its intrinsic
//! dimensions) or [`ImageState::Missing`] (the file could not be read / decoded —
//! the calm placeholder is drawn instead, and the failure is remembered so a
//! missing image is not re-attempted every frame).
//!
//! DECODE IS O(VISIBLE), NEVER O(DOC): the aggregator only calls [`Self::ensure`]
//! for images whose reserved rows fall in the visible band; an off-screen image is
//! never decoded. On decode the image is DOWNSCALED to its fit-to-column display
//! width (clamped to the device's `max_texture_dimension_2d`) so a huge source PNG
//! never uploads huge VRAM. [`Self::retain_paths`] prunes entries not referenced by
//! the current document each reshape, bounding the cache to the open doc's images.
//!
//! Declared native-only (`#[cfg(not(target_arch = "wasm32"))]` at the `mod` site,
//! like `daemon`/`session`), so every item below is unconditional within it — the
//! fs/decode reads never compile on wasm at all.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A decoded image's GPU residency + intrinsic size, or a remembered failure.
pub(crate) enum ImageState {
    Ready {
        #[allow(dead_code)] // RAII owner of the texture the `view` samples.
        texture: wgpu::Texture,
        view: wgpu::TextureView,
        /// The image's INTRINSIC (pre-downscale) pixel dimensions. Read by the
        /// decode test (asserting a bundled fixture's real size) + kept for future
        /// draw-side use (e.g. a natural-size cap); not read on the draw path today.
        #[allow(dead_code)]
        intrinsic: (u32, u32),
    },
    /// The file was absent / unreadable / not a decodable image.
    Missing,
}

/// One cache entry: the mtime it was decoded at (for staleness) + its state.
struct Entry {
    mtime_ns: u64,
    state: ImageState,
}

/// The decode cache. One entry per canonical path (staleness by stored mtime).
#[derive(Default)]
pub(crate) struct ImageCache {
    map: HashMap<PathBuf, Entry>,
}

/// The file's modification time as nanoseconds since the Unix epoch, or `0` when
/// the file is missing / its mtime is unreadable (which also drives the Missing
/// state below). PURE given the `metadata` result; split out for testability.
pub(crate) fn mtime_ns(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// The PURE upload-size decision: an image decoded for display is downscaled to
/// its fit-to-column display WIDTH (never UPSCALED past its intrinsic size — that
/// only wastes VRAM), preserving aspect, then BOTH dimensions clamped to the
/// device texture limit `max_dim`. Never zero (a 1px floor). Shared by the decode
/// path + its unit test so the two can't drift.
pub(crate) fn upload_size(intrinsic: (u32, u32), display_w: f32, max_dim: u32) -> (u32, u32) {
    let (iw, ih) = (intrinsic.0.max(1), intrinsic.1.max(1));
    let target_w = (display_w.ceil() as u32).clamp(1, iw); // never upscale
    // Preserve aspect: h = w * ih / iw (u64 math avoids overflow on big images).
    let target_h = ((target_w as u64 * ih as u64) / iw as u64).max(1) as u32;
    let cap = max_dim.max(1);
    // If either axis exceeds the device limit, scale the whole thing down to fit.
    if target_w <= cap && target_h <= cap {
        return (target_w.max(1), target_h.max(1));
    }
    let sx = cap as f32 / target_w as f32;
    let sy = cap as f32 / target_h as f32;
    let s = sx.min(sy);
    (
        ((target_w as f32 * s) as u32).clamp(1, cap),
        ((target_h as f32 * s) as u32).clamp(1, cap),
    )
}

impl ImageCache {
    /// Ensure `resolved` (an already doc-relative-resolved path) is decoded +
    /// uploaded, returning its current [`ImageState`]. A cache HIT (same path, same
    /// mtime) returns the stored state with no fs/decode work; a stale or absent
    /// entry decodes at `display_w` (downscaled + clamped to `max_dim`). A read /
    /// decode failure stores + returns [`ImageState::Missing`] (remembered, so it
    /// is not retried every frame until the mtime changes).
    ///
    /// The canonical path is the cache KEY (buffer-swap-safe identity); a path that
    /// cannot be canonicalized (a missing file) keys on the resolved path as-is and
    /// resolves to Missing.
    pub(crate) fn ensure(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resolved: &Path,
        display_w: f32,
        max_dim: u32,
    ) -> &ImageState {
        let key = std::fs::canonicalize(resolved).unwrap_or_else(|_| resolved.to_path_buf());
        let now = mtime_ns(&key);
        let fresh = self.map.get(&key).is_some_and(|e| e.mtime_ns == now);
        if !fresh {
            let state = decode_upload(device, queue, resolved, display_w, max_dim);
            self.map.insert(key.clone(), Entry { mtime_ns: now, state });
        }
        &self.map.get(&key).expect("just inserted").state
    }

    /// Prune every entry whose canonical path is NOT in `keep` — the current
    /// document's resolved+canonicalized image paths. Bounds the cache to the open
    /// doc's images (buffer-swap-safe), so switching documents frees the previous
    /// doc's decoded textures. An empty `keep` clears the cache entirely.
    pub(crate) fn retain_paths(&mut self, keep: &std::collections::HashSet<PathBuf>) {
        self.map.retain(|k, _| keep.contains(k));
    }

    /// Canonicalize a resolved path the SAME way [`Self::ensure`] keys it, so a
    /// caller can build the `keep` set [`Self::retain_paths`] compares against.
    pub(crate) fn canonical_key(resolved: &Path) -> PathBuf {
        std::fs::canonicalize(resolved).unwrap_or_else(|_| resolved.to_path_buf())
    }

    /// The decoded texture VIEW for `key` (a [`Self::canonical_key`]), if the entry
    /// is present + [`ImageState::Ready`]. Used by the draw pass to build the quad
    /// pipeline's bind groups AFTER all decoding (a distinct immutable borrow from
    /// the mutable [`Self::ensure`] pass), so the two never overlap.
    pub(crate) fn view(&self, key: &Path) -> Option<&wgpu::TextureView> {
        match self.map.get(key) {
            Some(Entry {
                state: ImageState::Ready { view, .. },
                ..
            }) => Some(view),
            _ => None,
        }
    }
}

/// Decode `resolved` to `Rgba8UnormSrgb`, downscaled to `display_w` (clamped to
/// `max_dim`), and upload it to a fresh texture. Returns [`ImageState::Missing`]
/// on any open / decode failure. PNG-only (the `image` crate's only enabled
/// feature); a non-PNG file simply fails to decode → Missing.
fn decode_upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    resolved: &Path,
    display_w: f32,
    max_dim: u32,
) -> ImageState {
    let decoded = image::ImageReader::open(resolved)
        .ok()
        .and_then(|rd| rd.with_guessed_format().ok())
        .and_then(|rd| rd.decode().ok());
    let Some(img) = decoded else {
        return ImageState::Missing;
    };
    let intrinsic = (img.width(), img.height());
    let (uw, uh) = upload_size(intrinsic, display_w, max_dim);
    // Downscale only when we actually want fewer pixels than the source (a small
    // image drawn at its own size uploads verbatim). Triangle filter = a calm,
    // slightly-soft downscale, appropriate for a quiet inline preview.
    let rgba = if uw < intrinsic.0 || uh < intrinsic.1 {
        image::imageops::resize(&img.to_rgba8(), uw, uh, image::imageops::FilterType::Triangle)
    } else {
        img.to_rgba8()
    };
    let (tw, th) = (rgba.width(), rgba.height());
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("inline image texture"),
        size: wgpu::Extent3d {
            width: tw.max(1),
            height: th.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            // Rgba8 = 4 bytes/px; the image buffer is tightly packed.
            bytes_per_row: Some(tw * 4),
            rows_per_image: Some(th),
        },
        wgpu::Extent3d {
            width: tw,
            height: th,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    ImageState::Ready {
        texture,
        view,
        intrinsic,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real headless device+queue, or `None` on a GPU-less machine (skip).
    #[cfg(not(target_arch = "wasm32"))]
    fn try_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        pollster::block_on(async {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .ok()?;
            adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("awl image-cache test device"),
                    ..Default::default()
                })
                .await
                .ok()
        })
    }

    /// The pure upload-size math: downscale to the display width, never upscale
    /// past intrinsic, preserve aspect, clamp to the device limit.
    #[test]
    fn upload_size_downscales_preserves_aspect_and_clamps() {
        // A 4000x2000 source fit to a 700px column → 700x350 (aspect kept).
        assert_eq!(upload_size((4000, 2000), 700.0, 16384), (700, 350));
        // Never UPSCALE: a 120x48 source asked to show at 700px stays 120x48.
        assert_eq!(upload_size((120, 48), 700.0, 16384), (120, 48));
        // The device limit clamps the whole thing down, aspect preserved.
        let (w, h) = upload_size((10000, 5000), 9000.0, 4096);
        assert!(w <= 4096 && h <= 4096, "clamped to the device limit");
        assert!((w as f32 / h as f32 - 2.0).abs() < 0.05, "aspect ~2:1 kept");
        // Never zero.
        assert_eq!(upload_size((1, 1), 0.0, 16384), (1, 1));
    }

    /// A missing / unreadable file decodes to the Missing state (the calm
    /// placeholder path), never a panic. Needs a real device; skipped without one.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn missing_file_decodes_to_missing_state() {
        let Some((device, queue)) = try_device() else {
            eprintln!("skipping missing_file_decodes_to_missing_state: no wgpu adapter");
            return;
        };
        let mut cache = ImageCache::default();
        let path = std::path::Path::new("/no/such/awl-image-does-not-exist.png");
        let st = cache.ensure(&device, &queue, path, 300.0, 16384);
        assert!(matches!(st, ImageState::Missing), "absent file → Missing");
    }

    /// A bundled real PNG decodes to Ready and reports its intrinsic dims. The
    /// `samples/tiny.png` fixture is 120x48.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn bundled_png_decodes_ready_with_intrinsic_dims() {
        let Some((device, queue)) = try_device() else {
            eprintln!("skipping bundled_png_decodes_ready_with_intrinsic_dims: no wgpu adapter");
            return;
        };
        let mut cache = ImageCache::default();
        let path = std::path::Path::new("samples/tiny.png");
        if std::fs::metadata(path).is_err() {
            eprintln!("skipping: samples/tiny.png fixture not present");
            return;
        }
        match cache.ensure(&device, &queue, path, 300.0, 16384) {
            ImageState::Ready { intrinsic, .. } => {
                assert_eq!(*intrinsic, (120, 48), "fixture intrinsic dims");
            }
            ImageState::Missing => panic!("bundled fixture must decode Ready"),
        }
        // A second ensure at the same mtime is a cache HIT (no re-decode / no panic).
        assert!(matches!(
            cache.ensure(&device, &queue, path, 300.0, 16384),
            ImageState::Ready { .. }
        ));
    }
}
