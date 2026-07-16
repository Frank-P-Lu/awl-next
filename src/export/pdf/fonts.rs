//! The closed PDF font world: four repository-owned OFL subsets, and no system
//! database. The exact files are embedded both in awl and in every PDF.

use std::collections::BTreeMap;

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

pub(super) fn has_glyph(role: FontRole, ch: char) -> bool {
    Face::parse(asset(role).bytes, 0)
        .ok()
        .and_then(|face| face.glyph_index(ch))
        .is_some()
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

pub(super) fn glyph_widths(role: FontRole) -> Vec<u16> {
    let face = Face::parse(asset(role).bytes, 0).expect("verified bundled PDF face");
    let upm = u32::from(face.units_per_em());
    (0..face.number_of_glyphs())
        .map(|id| {
            let raw = u32::from(face.glyph_hor_advance(GlyphId(id)).unwrap_or(0));
            ((raw * 1000 + upm / 2) / upm) as u16
        })
        .collect()
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
            assert!(
                parsed.is_subsetting_allowed(),
                "{} subsetting",
                face.pdf_name
            );
            assert!(parsed.tables().cmap.is_some(), "{} cmap", face.pdf_name);
            assert!(parsed.tables().glyf.is_some(), "{} glyf", face.pdf_name);
            assert!(parsed.tables().hmtx.is_some(), "{} hmtx", face.pdf_name);
        }
        let inventory = include_str!("../../../assets/fonts/LICENSES.md");
        let ofl = include_str!("../../../assets/fonts/OFL.txt");
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
}
