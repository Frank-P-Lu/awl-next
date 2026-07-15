//! A hand-rolled, byte-DETERMINISTIC ZIP writer — STORED entries only (no
//! compression), all timestamps ZEROED, no extra fields. This is the whole of
//! awl's `.docx` container machinery: an OOXML package is just a ZIP with a
//! fixed set of XML parts, so the export needs a writer, not a dependency. The
//! entries are STORED (method 0) so the archive's bytes are a pure function of
//! the part names + contents — the same document always exports the same bytes,
//! which is exactly what the golden-file gate asserts (and what a compressed
//! stream, with its implementation-defined deflate tables, could never promise).
//!
//! DELIBERATELY MINIMAL: no Zip64, no data descriptors, no UTF-8 flag juggling
//! (every part name we emit is ASCII), no compression. The format is the classic
//! local-header + central-directory + end-of-central-directory triple, laid out
//! in the order entries were added. Everything here is pure byte-pushing, so it
//! compiles + runs identically on native and `wasm32`.

/// The IEEE CRC-32 of `data` (polynomial `0xEDB88320`, the ZIP/PNG standard).
/// Table-free (a per-byte bit loop) — an export writes a handful of small parts,
/// so a lookup table's setup would dwarf the work; correctness + zero shared
/// state is what matters here. Verified against the known
/// `"123456789" -> 0xCBF43926` check value in the tests.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// One STORED entry queued for the archive.
struct Entry {
    name: String,
    size: u32,
    crc: u32,
    offset: u32,
}

/// A builder for a deterministic STORED ZIP. Add parts with [`Self::add`] in the
/// order they should appear, then [`Self::finish`] for the archive bytes.
#[derive(Default)]
pub struct ZipWriter {
    body: Vec<u8>,
    entries: Vec<Entry>,
}

impl ZipWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a STORED entry. `name` must be an ASCII package path
    /// (`[Content_Types].xml`, `word/document.xml`, …); `data` is written
    /// verbatim, uncompressed.
    pub fn add(&mut self, name: &str, data: &[u8]) {
        let crc = crc32(data);
        let offset = self.body.len() as u32;
        let size = data.len() as u32;
        self.write_local_header(name, crc, size);
        self.body.extend_from_slice(data);
        self.entries.push(Entry { name: name.to_string(), size, crc, offset });
    }

    fn write_local_header(&mut self, name: &str, crc: u32, size: u32) {
        let b = &mut self.body;
        b.extend_from_slice(&0x0403_4b50u32.to_le_bytes()); // local file header sig
        b.extend_from_slice(&20u16.to_le_bytes()); // version needed
        b.extend_from_slice(&0u16.to_le_bytes()); // flags
        b.extend_from_slice(&0u16.to_le_bytes()); // method 0 = stored
        b.extend_from_slice(&0u16.to_le_bytes()); // mod time (zeroed)
        b.extend_from_slice(&0u16.to_le_bytes()); // mod date (zeroed)
        b.extend_from_slice(&crc.to_le_bytes());
        b.extend_from_slice(&size.to_le_bytes()); // compressed size == size (stored)
        b.extend_from_slice(&size.to_le_bytes()); // uncompressed size
        b.extend_from_slice(&(name.len() as u16).to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes()); // extra len
        b.extend_from_slice(name.as_bytes());
    }

    /// Serialize the whole archive: the accumulated local records, then the
    /// central directory, then the end-of-central-directory record.
    pub fn finish(self) -> Vec<u8> {
        let ZipWriter { mut body, entries } = self;
        let cd_offset = body.len() as u32;
        for e in &entries {
            body.extend_from_slice(&0x0201_4b50u32.to_le_bytes()); // central dir header sig
            body.extend_from_slice(&20u16.to_le_bytes()); // version made by
            body.extend_from_slice(&20u16.to_le_bytes()); // version needed
            body.extend_from_slice(&0u16.to_le_bytes()); // flags
            body.extend_from_slice(&0u16.to_le_bytes()); // method 0 = stored
            body.extend_from_slice(&0u16.to_le_bytes()); // mod time
            body.extend_from_slice(&0u16.to_le_bytes()); // mod date
            body.extend_from_slice(&e.crc.to_le_bytes());
            body.extend_from_slice(&e.size.to_le_bytes());
            body.extend_from_slice(&e.size.to_le_bytes());
            body.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
            body.extend_from_slice(&0u16.to_le_bytes()); // extra len
            body.extend_from_slice(&0u16.to_le_bytes()); // comment len
            body.extend_from_slice(&0u16.to_le_bytes()); // disk number start
            body.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            body.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            body.extend_from_slice(&e.offset.to_le_bytes()); // local header offset
            body.extend_from_slice(e.name.as_bytes());
        }
        let cd_size = body.len() as u32 - cd_offset;
        let count = entries.len() as u16;
        body.extend_from_slice(&0x0605_4b50u32.to_le_bytes()); // end of central dir sig
        body.extend_from_slice(&0u16.to_le_bytes()); // this disk
        body.extend_from_slice(&0u16.to_le_bytes()); // cd start disk
        body.extend_from_slice(&count.to_le_bytes()); // entries this disk
        body.extend_from_slice(&count.to_le_bytes()); // entries total
        body.extend_from_slice(&cd_size.to_le_bytes());
        body.extend_from_slice(&cd_offset.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes()); // comment len
        body
    }
}
