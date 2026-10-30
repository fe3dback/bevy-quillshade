//! Material's icons (decision 0013): asset kinds, node categories and preview modes, as [`IconRef`] constants over
//! the open icon set of `editor_core::ui::icons`. Names must stay disjoint from the core [`IconId`] set.

use editor_core::ui::icons::{Icon, IconId, IconRef, Kind, custom};
use shader_graph_core::model::{AssetKind, MaterialKind};
use shader_project::{AssetId, AssetType};

const MATERIAL_UNLIT_ICON: Icon = custom("material-unlit", Kind::Solid, include_str!("../icons/material-unlit.svg"));
/// The letters "rs" as strokes: the generated Rust crate.
const RUST_RS_ICON: Icon = custom("rust-rs", Kind::Line, include_str!("../icons/rust_rs.svg"));

pub const FUNCTION: IconRef = IconRef::new(&icon_rs::VARIABLES);
pub const PBR: IconRef = IconRef::new(&icon_rs::MATERIAL_LIBRARY);
pub const UNLIT: IconRef = IconRef::new(&MATERIAL_UNLIT_ICON);
/// A curve asset: `curve_kit`'s icon.
pub const CURVE: IconRef = curve_kit::editor::CURVE_ICON;
pub const CONSTS: IconRef = IconRef::new(&icon_rs::CONFIGURATIONS);
/// A Texture asset (design §6.1): `texture_kit`'s icon; game images keep `IconId::Image`.
pub const TEXTURE: IconRef = texture_kit::editor::TEXTURE_ICON;
/// The node preview's Full mode (every output slot, decision 0022); Unlit / Lit use [`UNLIT`] / [`PBR`].
pub const LAYERS: IconRef = IconRef::new(&icon_rs::LAYERS);
/// The generated Rust crate ("Show generated Rust"); generated WESL keeps `IconId::Code`.
pub const RUST: IconRef = IconRef::new(&RUST_RS_ICON);
/// A gradient collection (gradient design §8).
pub const GRADIENTS: IconRef = IconRef::new(&icon_rs::APPEARANCE);
/// A Sprite2d material: the same picture as a game image (`IconId::Image`), so it is a core icon, not a constant.
pub fn sprite() -> IconRef {
    IconId::Image.into()
}

/// Every icon declared here, for the rasterize test.
pub const MATERIAL_ICONS: &[IconRef] = &[FUNCTION, PBR, UNLIT, CURVE, CONSTS, TEXTURE, LAYERS, RUST, GRADIENTS];

/// The icon of an asset of `kind` (`None`: unreadable file).
pub fn for_asset(kind: Option<AssetKind>) -> IconRef {
    match kind {
        Some(AssetKind::Function) => FUNCTION,
        Some(AssetKind::Material(MaterialKind::Pbr3d(_))) => PBR,
        Some(AssetKind::Material(MaterialKind::Unlit3d(_))) => UNLIT,
        Some(AssetKind::Material(MaterialKind::Sprite2d(_))) => sprite(),
        None => IconId::File.into(),
    }
}

/// The icon of asset `id`; `kind` is its graph kind when it is a loaded graph.
pub fn for_id(id: &AssetId, kind: Option<AssetKind>) -> IconRef {
    match id.ty {
        AssetType::Curve => CURVE,
        AssetType::Gradients => GRADIENTS,
        AssetType::Consts => CONSTS,
        AssetType::Texture => TEXTURE,
        AssetType::Material | AssetType::Function => for_asset(kind),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_icons_rasterize() {
        editor_core::ui::icons::assert_icons_rasterize(MATERIAL_ICONS);
    }

    #[test]
    fn custom_icons_use_only_palette_colours() {
        editor_core::ui::icons::assert_only_palette_colours(&[MATERIAL_UNLIT_ICON, RUST_RS_ICON]);
    }

    #[test]
    fn material_and_core_icon_names_are_disjoint() {
        for icon in MATERIAL_ICONS {
            for id in IconId::ALL {
                assert_ne!(icon.name(), IconRef::from(id).name(), "{id:?} clashes with a Material icon");
            }
        }
        let mut names: Vec<_> = MATERIAL_ICONS.iter().map(|i| i.name()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), MATERIAL_ICONS.len());
    }

    /// Design §6.1: Texture assets have their own icon; game images keep the image one.
    #[test]
    fn texture_icon_differs_from_image() {
        assert_ne!(TEXTURE, IconId::Image.into());
        let texture = AssetId::new(AssetType::Texture, "textures/rock");
        assert_eq!(for_id(&texture, None), TEXTURE);
    }
}
