//! The closed PDF font world: four repository-owned OFL faces and no system
//! database. Shaping uses the complete faces; embedding rebuilds each TrueType
//! face with only this document's glyph outlines (plus composite dependencies).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use glyphon::{FontSystem, fontdb};
use ttf_parser::{Face, GlyphId, Permissions};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum FontRole {
    Serif,
    SerifBold,
    Mono,
    MonoBold,
}

pub(super) const ROLES: [FontRole; 4] = [
    FontRole::Serif,
    FontRole::SerifBold,
    FontRole::Mono,
    FontRole::MonoBold,
];

pub(super) struct FontAsset {
    pub role: FontRole,
    pub family: &'static str,
    pub pdf_name: &'static str,
    pub bytes: &'static [u8],
    pub weight: u16,
}

pub(super) const ASSETS: [FontAsset; 4] = [
    FontAsset {
        role: FontRole::Serif,
        family: "Bitter",
        pdf_name: "AWLBitter-Regular",
        bytes: include_bytes!("../../../assets/fonts/Bitter-Regular.ttf"),
        weight: 400,
    },
    FontAsset {
        role: FontRole::SerifBold,
        family: "Bitter",
        pdf_name: "AWLBitter-Bold",
        bytes: include_bytes!("../../../assets/fonts/Bitter-Bold.ttf"),
        weight: 700,
    },
    FontAsset {
        role: FontRole::Mono,
        family: "IBM Plex Mono",
        pdf_name: "AWLIBMPlexMono-Light",
        bytes: include_bytes!("../../../assets/fonts/IBMPlexMono-Light.ttf"),
        weight: 300,
    },
    FontAsset {
        role: FontRole::MonoBold,
        family: "IBM Plex Mono",
        pdf_name: "AWLIBMPlexMono-Bold",
        bytes: include_bytes!("../../../assets/fonts/IBMPlexMono-Bold.ttf"),
        weight: 700,
    },
];

/// The immutable source faces behind the PDF coverage checks. Parsing a
/// TrueType directory for every scalar made ordinary export text needlessly pay
/// the font-load cost; the bundled bytes are static, so one parsed face per role
/// is sufficient for the process lifetime.
static FACES: LazyLock<[Face<'static>; 4]> = LazyLock::new(|| {
    std::array::from_fn(|index| {
        let asset = &ASSETS[index];
        Face::parse(asset.bytes, 0).expect("verified bundled PDF face")
    })
});

pub(super) struct Fonts {
    pub system: FontSystem,
    ids: BTreeMap<FontRole, fontdb::ID>,
}

impl Fonts {
    pub fn new() -> Self {
        let mut db = fontdb::Database::new();
        let mut ids = BTreeMap::new();
        for asset in &ASSETS {
            assert!(
                embedding_is_permitted(asset),
                "bundled PDF font {} does not permit outline embedding",
                asset.pdf_name
            );
            let loaded = db.load_font_source(fontdb::Source::Binary(std::sync::Arc::new(
                asset.bytes.to_vec(),
            )));
            let id = *loaded.first().expect("bundled PDF font must parse");
            ids.insert(asset.role, id);
        }
        db.set_serif_family("Bitter");
        db.set_monospace_family("IBM Plex Mono");
        Self {
            system: FontSystem::new_with_locale_and_db("en-US".into(), db),
            ids,
        }
    }

    pub fn role_for_id(&self, id: fontdb::ID) -> Option<FontRole> {
        self.ids
            .iter()
            .find_map(|(role, known)| (*known == id).then_some(*role))
    }
}

pub(super) fn asset(role: FontRole) -> &'static FontAsset {
    &ASSETS[role_index(role)]
}

pub(super) const fn role_index(role: FontRole) -> usize {
    match role {
        FontRole::Serif => 0,
        FontRole::SerifBold => 1,
        FontRole::Mono => 2,
        FontRole::MonoBold => 3,
    }
}

fn face(role: FontRole) -> &'static Face<'static> {
    &FACES[role_index(role)]
}

pub(super) fn has_glyph(role: FontRole, ch: char) -> bool {
    face(role).glyph_index(ch).is_some()
}

/// Fixed, representative coverage probe for the CLI micro-benchmark. Keeping
/// this beside [`has_glyph`] means the benchmark cannot drift to a different
/// lookup than PDF export actually performs.
pub(super) fn glyph_probe() -> usize {
    const SCALARS: &str = "The quick brown fox — café 123 []{}() 😀 🦉";
    ROLES
        .iter()
        .copied()
        .map(|role| SCALARS.chars().filter(|ch| has_glyph(role, *ch)).count())
        .sum()
}

pub(super) fn fallback_char(role: FontRole) -> char {
    if has_glyph(role, '\u{25a1}') {
        '\u{25a1}'
    } else {
        '?'
    }
}

pub(super) fn embedding_is_permitted(asset: &FontAsset) -> bool {
    let Ok(face) = Face::parse(asset.bytes, 0) else {
        return false;
    };
    face.permissions() == Some(Permissions::Installable) && face.is_outline_embedding_allowed()
}

pub(super) fn glyph_widths(role: FontRole, glyphs: &BTreeSet<u16>) -> Vec<(u16, u16)> {
    let face = Face::parse(asset(role).bytes, 0).expect("verified bundled PDF face");
    let upm = u32::from(face.units_per_em());
    glyphs
        .iter()
        .copied()
        .map(|id| {
            let raw = u32::from(face.glyph_hor_advance(GlyphId(id)).unwrap_or(0));
            (id, ((raw * 1000 + upm / 2) / upm) as u16)
        })
        .collect()
}

/// Build a PDF-safe TrueType subset while preserving original glyph IDs.
pub(super) fn subset(role: FontRole, used: &BTreeSet<u16>) -> Vec<u8> {
    let source = asset(role).bytes;
    let tables = table_directory(source);
    let head = table(source, &tables, b"head");
    let maxp = table(source, &tables, b"maxp");
    let loca = table(source, &tables, b"loca");
    let glyf = table(source, &tables, b"glyf");
    let glyph_count = be_u16(maxp, 4);
    let long_loca = be_i16(head, 50) == 1;
    let offsets = (0..=glyph_count)
        .map(|id| {
            if long_loca {
                be_u32(loca, usize::from(id) * 4)
            } else {
                u32::from(be_u16(loca, usize::from(id) * 2)) * 2
            }
        })
        .collect::<Vec<_>>();

    let mut included = used
        .iter()
        .copied()
        .filter(|id| *id < glyph_count)
        .collect::<BTreeSet<_>>();
    included.insert(0);
    let mut pending = included.iter().copied().collect::<Vec<_>>();
    while let Some(id) = pending.pop() {
        let bytes = glyph_bytes(glyf, &offsets, id);
        if bytes.len() < 10 || be_i16(bytes, 0) >= 0 {
            continue;
        }
        let mut cursor = 10;
        loop {
            assert!(cursor + 4 <= bytes.len(), "truncated composite glyph {id}");
            let flags = be_u16(bytes, cursor);
            let component = be_u16(bytes, cursor + 2);
            if included.insert(component) {
                pending.push(component);
            }
            cursor += 4;
            cursor += if flags & 0x0001 != 0 { 4 } else { 2 };
            cursor += if flags & 0x0008 != 0 {
                2
            } else if flags & 0x0040 != 0 {
                4
            } else if flags & 0x0080 != 0 {
                8
            } else {
                0
            };
            if flags & 0x0020 == 0 {
                break;
            }
        }
    }

    let mut subset_glyf = Vec::new();
    let mut subset_loca = Vec::with_capacity((usize::from(glyph_count) + 1) * 4);
    for id in 0..glyph_count {
        subset_loca.extend_from_slice(&(subset_glyf.len() as u32).to_be_bytes());
        if included.contains(&id) {
            subset_glyf.extend_from_slice(glyph_bytes(glyf, &offsets, id));
            while subset_glyf.len() % 4 != 0 {
                subset_glyf.push(0);
            }
        }
    }
    subset_loca.extend_from_slice(&(subset_glyf.len() as u32).to_be_bytes());

    let mut subset_head = head.to_vec();
    subset_head[8..12].fill(0);
    subset_head[50..52].copy_from_slice(&1i16.to_be_bytes());
    let mut kept = Vec::new();
    for tag in [
        *b"cvt ", *b"fpgm", *b"glyf", *b"head", *b"hhea", *b"hmtx", *b"loca", *b"maxp", *b"prep",
    ] {
        let bytes = match &tag {
            b"glyf" => subset_glyf.clone(),
            b"head" => subset_head.clone(),
            b"loca" => subset_loca.clone(),
            _ => tables
                .get(&tag)
                .map(|&(offset, length)| source[offset..offset + length].to_vec())
                .unwrap_or_default(),
        };
        if !bytes.is_empty() {
            kept.push((tag, bytes));
        }
    }
    build_sfnt(kept)
}

fn table_directory(bytes: &[u8]) -> BTreeMap<[u8; 4], (usize, usize)> {
    let count = usize::from(be_u16(bytes, 4));
    (0..count)
        .map(|index| {
            let at = 12 + index * 16;
            let tag = bytes[at..at + 4].try_into().unwrap();
            let offset = be_u32(bytes, at + 8) as usize;
            let length = be_u32(bytes, at + 12) as usize;
            assert!(offset + length <= bytes.len(), "invalid TrueType table");
            (tag, (offset, length))
        })
        .collect()
}

fn table<'a>(
    bytes: &'a [u8],
    tables: &BTreeMap<[u8; 4], (usize, usize)>,
    tag: &[u8; 4],
) -> &'a [u8] {
    let &(offset, length) = tables.get(tag).expect("required TrueType table");
    &bytes[offset..offset + length]
}

fn glyph_bytes<'a>(glyf: &'a [u8], offsets: &[u32], id: u16) -> &'a [u8] {
    let start = offsets[usize::from(id)] as usize;
    let end = offsets[usize::from(id) + 1] as usize;
    assert!(start <= end && end <= glyf.len(), "invalid glyph location");
    &glyf[start..end]
}

fn build_sfnt(tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    let count = tables.len() as u16;
    let power = 1u16 << (15 - count.leading_zeros() as u16);
    let search_range = power * 16;
    let entry_selector = power.trailing_zeros() as u16;
    let range_shift = count * 16 - search_range;
    let data_start = 12 + tables.len() * 16;
    let mut offsets = Vec::with_capacity(tables.len());
    let mut cursor = data_start;
    for (_, bytes) in &tables {
        offsets.push(cursor);
        cursor += (bytes.len() + 3) & !3;
    }
    let mut out = Vec::with_capacity(cursor);
    out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    out.extend_from_slice(&count.to_be_bytes());
    out.extend_from_slice(&search_range.to_be_bytes());
    out.extend_from_slice(&entry_selector.to_be_bytes());
    out.extend_from_slice(&range_shift.to_be_bytes());
    let mut head_offset = None;
    for ((tag, bytes), offset) in tables.iter().zip(&offsets) {
        out.extend_from_slice(tag);
        out.extend_from_slice(&checksum(bytes).to_be_bytes());
        out.extend_from_slice(&(*offset as u32).to_be_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        if tag == b"head" {
            head_offset = Some(*offset);
        }
    }
    for (_, bytes) in &tables {
        out.extend_from_slice(bytes);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    let adjustment = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
    let at = head_offset.expect("subset head table") + 8;
    out[at..at + 4].copy_from_slice(&adjustment.to_be_bytes());
    out
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes
        .chunks(4)
        .map(|chunk| {
            let mut word = [0; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            u32::from_be_bytes(word)
        })
        .fold(0, u32::wrapping_add)
}

fn be_u16(bytes: &[u8], at: usize) -> u16 {
    u16::from_be_bytes(bytes[at..at + 2].try_into().unwrap())
}

fn be_i16(bytes: &[u8], at: usize) -> i16 {
    i16::from_be_bytes(bytes[at..at + 2].try_into().unwrap())
}

fn be_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap())
}

pub(super) struct Descriptor {
    pub bbox: [i32; 4],
    pub ascent: i32,
    pub descent: i32,
    pub cap_height: i32,
}

pub(super) fn descriptor(role: FontRole) -> Descriptor {
    let face = Face::parse(asset(role).bytes, 0).expect("verified bundled PDF face");
    let upm = i32::from(face.units_per_em());
    let scale = |v: i16| i32::from(v) * 1000 / upm;
    let b = face.global_bounding_box();
    Descriptor {
        bbox: [
            scale(b.x_min),
            scale(b.y_min),
            scale(b.x_max),
            scale(b.y_max),
        ],
        ascent: scale(face.ascender()),
        descent: scale(face.descender()),
        cap_height: scale(face.capital_height().unwrap_or(face.ascender())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct SegmentCount(usize);

    impl ttf_parser::OutlineBuilder for SegmentCount {
        fn move_to(&mut self, _: f32, _: f32) {
            self.0 += 1;
        }
        fn line_to(&mut self, _: f32, _: f32) {
            self.0 += 1;
        }
        fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
            self.0 += 1;
        }
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
            self.0 += 1;
        }
        fn close(&mut self) {
            self.0 += 1;
        }
    }

    #[test]
    fn pdf_faces_are_true_type_installable_ofl_inventory_faces() {
        for face in &ASSETS {
            let parsed = Face::parse(face.bytes, 0).expect(face.pdf_name);
            assert_eq!(
                parsed.permissions(),
                Some(Permissions::Installable),
                "{} fsType",
                face.pdf_name
            );
            assert!(
                embedding_is_permitted(face),
                "{} outline embedding",
                face.pdf_name
            );
            // Subsetting is both technically supported by these `glyf` faces and
            // permitted by their installable-embedding license bits.
            assert!(
                parsed.is_subsetting_allowed(),
                "{} subsetting permitted by license",
                face.pdf_name
            );
            assert!(parsed.tables().cmap.is_some(), "{} cmap", face.pdf_name);
            assert!(parsed.tables().glyf.is_some(), "{} glyf", face.pdf_name);
            assert!(parsed.tables().hmtx.is_some(), "{} hmtx", face.pdf_name);
        }
        let inventory = crate::embedded_docs::FONT_LICENSES_MD;
        let ofl = crate::embedded_docs::FONT_OFL_TXT;
        for file in [
            "Bitter-Regular.ttf",
            "Bitter-Bold.ttf",
            "IBMPlexMono-Light.ttf",
            "IBMPlexMono-Bold.ttf",
        ] {
            assert!(
                inventory.contains(file),
                "missing inventory record for {file}"
            );
        }
        assert!(ofl.contains("SIL OPEN FONT LICENSE Version 1.1"));
    }

    #[test]
    fn cached_coverage_lookup_matches_every_bundled_face() {
        for asset in &ASSETS {
            let parsed = Face::parse(asset.bytes, 0).expect(asset.pdf_name);
            for ch in "Awl café — []{}() 😀 🦉\n".chars() {
                assert_eq!(
                    has_glyph(asset.role, ch),
                    parsed.glyph_index(ch).is_some(),
                    "{} coverage for {ch:?}",
                    asset.pdf_name
                );
            }
        }
    }

    #[test]
    fn subsets_preserve_composite_outlines_and_sfnt_checksum() {
        for asset in &ASSETS {
            let source = Face::parse(asset.bytes, 0).unwrap();
            let id = source.glyph_index('é').expect("PDF faces contain e-acute");
            let bytes = subset(asset.role, &BTreeSet::from([id.0]));
            assert_eq!(checksum(&bytes), 0xB1B0_AFBA, "{} checksum", asset.pdf_name);
            let subset = Face::parse(&bytes, 0).unwrap();
            let mut source_segments = SegmentCount::default();
            let mut subset_segments = SegmentCount::default();
            assert_eq!(
                source.outline_glyph(id, &mut source_segments),
                subset.outline_glyph(id, &mut subset_segments),
                "{} outline bounds",
                asset.pdf_name
            );
            assert_eq!(
                source_segments.0, subset_segments.0,
                "{} composite components",
                asset.pdf_name
            );
        }
    }
}
