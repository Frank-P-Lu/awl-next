use crate::export::model::{ExportImage, ImageMime, ImageSource};

const BASE: &str = "\
---
lang: en
title: excluded
---
# PDF Export Fixture

A paragraph with **bold**, *italic*, ~~struck~~, ==highlighted==, `inline code`, and **`bold mono`**.
It carries a [real link](https://example.com/path?q=1&r=2), a soft
break, and a hard break here.\x20\x20
After the hard break sits an unsupported owl 🦉.

## Heading Two
### Heading Three
#### Heading Four
##### Heading Five
###### Heading Six

- first bullet
- second bullet
  - nested bullet

3. ordered three
4. ordered four

- [ ] open task
- [x] done task

> A quoted paragraph with **strong quote**.
>
> - quoted list item

---

| Plain | Left | Center | Right |
|-------|:-----|:------:|------:|
| a | b | c | d |
| long cell | second | third | fourth |

```rust
fn main() {
    println!(\"fixture\");
}
```

![alpha picture|48](alpha.png)

![jpeg photo](photo.jpg)

![missing picture](missing.png)
";

pub(super) fn markdown() -> String {
    let mut out = BASE.to_string();
    for i in 0..44 {
        out.push_str(&format!(
            "\nPagination paragraph {i:02}: fixed measure text survives automatic page breaking with deterministic margins and leading.\n"
        ));
    }
    out
}

pub(super) fn png_rgba() -> [u8; 8] {
    [0x20, 0x40, 0x60, 0x80, 0x80, 0x60, 0x40, 0xff]
}

pub(super) fn png() -> Vec<u8> {
    crate::paste_image::encode_rgba_png(2, 1, &png_rgba()).expect("encode fixture PNG")
}

pub(super) fn jpeg() -> Vec<u8> {
    decode_base64(include_str!("../../testdata/tiny.jpg.b64"))
}

#[derive(Clone)]
pub(super) struct Images {
    png: Vec<u8>,
    jpeg: Vec<u8>,
}

impl Images {
    pub(super) fn new() -> Self {
        Self {
            png: png(),
            jpeg: jpeg(),
        }
    }
}

impl ImageSource for Images {
    fn resolve(&self, src: &str) -> Option<ExportImage> {
        match src {
            "alpha.png" => Some(ExportImage {
                bytes: self.png.clone(),
                width: 2,
                height: 1,
                mime: ImageMime::Png,
            }),
            "photo.jpg" => Some(ExportImage {
                bytes: self.jpeg.clone(),
                width: 120,
                height: 48,
                mime: ImageMime::Jpeg,
            }),
            _ => None,
        }
    }
}

pub(super) struct NoImages;

impl ImageSource for NoImages {
    fn resolve(&self, _src: &str) -> Option<ExportImage> {
        None
    }
}

fn decode_base64(text: &str) -> Vec<u8> {
    fn value(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let clean = text
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect::<Vec<_>>();
    assert_eq!(clean.len() % 4, 0);
    let mut out = Vec::with_capacity(clean.len() / 4 * 3);
    for chunk in clean.chunks_exact(4) {
        let a = value(chunk[0]).unwrap();
        let b = value(chunk[1]).unwrap();
        let c = (chunk[2] != b'=').then(|| value(chunk[2]).unwrap());
        let d = (chunk[3] != b'=').then(|| value(chunk[3]).unwrap());
        out.push((a << 2) | (b >> 4));
        if let Some(c) = c {
            out.push((b << 4) | (c >> 2));
            if let Some(d) = d {
                out.push((c << 6) | d);
            }
        }
    }
    out
}
