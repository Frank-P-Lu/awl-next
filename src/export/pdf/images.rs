//! Deterministic PDF image preparation. JPEG bytes pass through untouched;
//! PNG is decoded by the already-bundled pure-Rust decoder to RGB + alpha.

use image::ImageFormat;

use super::super::model::{ExportImage, ImageMime};

#[derive(Clone)]
pub(super) struct PdfImage {
    pub width: u32,
    pub height: u32,
    pub color_space: &'static str,
    pub data: Vec<u8>,
    pub alpha: Option<Vec<u8>>,
    pub jpeg: bool,
    pub alt: String,
    pub src: String,
}

impl PdfImage {
    pub fn prepare(image: ExportImage, src: &str, alt: &str) -> Option<Self> {
        match image.mime {
            ImageMime::Jpeg => Some(Self {
                width: image.width,
                height: image.height,
                color_space: jpeg_color_space(&image.bytes),
                data: image.bytes,
                alpha: None,
                jpeg: true,
                alt: alt.to_string(),
                src: src.to_string(),
            }),
            ImageMime::Png => {
                let decoded = image::load_from_memory_with_format(&image.bytes, ImageFormat::Png)
                    .ok()?
                    .into_rgba8();
                let (width, height) = decoded.dimensions();
                let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
                let mut alpha = Vec::with_capacity(width as usize * height as usize);
                let mut translucent = false;
                for pixel in decoded.pixels() {
                    rgb.extend_from_slice(&pixel.0[..3]);
                    alpha.push(pixel.0[3]);
                    translucent |= pixel.0[3] != 255;
                }
                Some(Self {
                    width,
                    height,
                    color_space: "/DeviceRGB",
                    data: rgb,
                    alpha: translucent.then_some(alpha),
                    jpeg: false,
                    alt: alt.to_string(),
                    src: src.to_string(),
                })
            }
        }
    }
}

fn jpeg_color_space(bytes: &[u8]) -> &'static str {
    if bytes.len() < 4 || !bytes.starts_with(&[0xff, 0xd8]) {
        return "/DeviceRGB";
    }
    let mut i = 2;
    while i + 9 < bytes.len() {
        if bytes[i] != 0xff {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        if marker == 0xff || (0xd0..=0xd9).contains(&marker) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        let sof =
            (0xc0..=0xcf).contains(&marker) && marker != 0xc4 && marker != 0xc8 && marker != 0xcc;
        if sof {
            return match bytes[i + 9] {
                1 => "/DeviceGray",
                4 => "/DeviceCMYK",
                _ => "/DeviceRGB",
            };
        }
        if len < 2 {
            break;
        }
        i += len + 2;
    }
    "/DeviceRGB"
}
