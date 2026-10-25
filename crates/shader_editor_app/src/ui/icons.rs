//! UI icons (decision 0013): icon-rs SVGs plus a few of our own (`icons/*.svg`), rasterized with resvg at the
//! icon's size in physical pixels, so they stay sharp at any window scale factor.
//!
//! Glyph and line icons are rendered in white and tinted through `ImageNode::color`; their accent detail is a
//! separate child layer in [`ACCENT`]. Solid and sketch icons keep the colours of icon-rs's dark palette with our
//! accent ([`ICON_PALETTE`]), the tint only fades them through its alpha.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;
use icon_rs::{Icon, Kind, Palette};
use resvg::{tiny_skia, usvg};
use shader_graph_core::model::{AssetKind, MaterialKind};
use shader_project::{AssetId, AssetType};

use super::{ACCENT, ACCENT_RGB};
use crate::state::NewKind;

/// Colours of solid and sketch icons: icon-rs's dark palette with the UI accent.
const ICON_PALETTE: Palette =
    Palette { accent: icon_rs::Rgb(ACCENT_RGB[0], ACCENT_RGB[1], ACCENT_RGB[2]), ..Palette::DARK };

// Our own icons, drawn by hand in icon-rs's house style (see icons/README.md). `svg_dark` is unused: the dark
// variant is made with `Icon::recolor`, as icon-rs makes its own.
const FOLDER_OPEN: Icon = custom("folder-open", Kind::Line, include_str!("../../icons/folder-open.svg"));
const MATERIAL_UNLIT: Icon = custom("material-unlit", Kind::Solid, include_str!("../../icons/material-unlit.svg"));
const SAVE: Icon = custom("save", Kind::Line, include_str!("../../icons/save.svg"));
const SAVE_ALL: Icon = custom("save-all", Kind::Line, include_str!("../../icons/save-all.svg"));
const FIT: Icon = custom("fit", Kind::Line, include_str!("../../icons/fit.svg"));
const AUTO_LAYOUT: Icon = custom("auto-layout", Kind::Line, include_str!("../../icons/auto-layout.svg"));

const fn custom(name: &'static str, kind: Kind, svg: &'static str) -> Icon {
    Icon { name, kind, title: name, svg, svg_dark: svg }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum IconId {
    #[default]
    File,
    Folder,
    FolderOpen,
    CaretRight,
    CaretDown,
    Function,
    Pbr,
    Unlit,
    Sprite,
    FolderNew,
    Edit,
    MoveToFolder,
    Delete,
    Code,
    Save,
    SaveAll,
    Regenerate,
    Undo,
    Redo,
    AutoLayout,
    Comment,
    Fit,
    Copy,
    OpenExternal,
    Curve,
    Close,
    Consts,
    Check,
    Plus,
    Info,
    Link,
    Help,
}

impl IconId {
    pub const ALL: [IconId; 32] = [
        IconId::File,
        IconId::Folder,
        IconId::FolderOpen,
        IconId::CaretRight,
        IconId::CaretDown,
        IconId::Function,
        IconId::Pbr,
        IconId::Unlit,
        IconId::Sprite,
        IconId::FolderNew,
        IconId::Edit,
        IconId::MoveToFolder,
        IconId::Delete,
        IconId::Code,
        IconId::Save,
        IconId::SaveAll,
        IconId::Regenerate,
        IconId::Undo,
        IconId::Redo,
        IconId::AutoLayout,
        IconId::Comment,
        IconId::Fit,
        IconId::Copy,
        IconId::OpenExternal,
        IconId::Curve,
        IconId::Close,
        IconId::Consts,
        IconId::Check,
        IconId::Plus,
        IconId::Info,
        IconId::Link,
        IconId::Help,
    ];

    fn source(self) -> Icon {
        match self {
            IconId::File => icon_rs::FILE,
            IconId::Folder => icon_rs::FOLDER,
            IconId::FolderOpen => FOLDER_OPEN,
            IconId::CaretRight => icon_rs::CARET_RIGHT_FILLED,
            IconId::CaretDown => icon_rs::CARET_DOWN_FILLED,
            IconId::Function => icon_rs::VARIABLES,
            IconId::Pbr => icon_rs::MATERIAL_LIBRARY,
            IconId::Unlit => MATERIAL_UNLIT,
            IconId::Sprite => icon_rs::IMAGE,
            IconId::FolderNew => icon_rs::FOLDER_NEW,
            IconId::Edit => icon_rs::EDIT,
            IconId::MoveToFolder => icon_rs::MOVE_TO_FOLDER,
            IconId::Delete => icon_rs::DELETE,
            IconId::Code => icon_rs::CODE,
            IconId::Save => SAVE,
            IconId::SaveAll => SAVE_ALL,
            IconId::Regenerate => icon_rs::SYNC_DOCUMENT,
            IconId::Undo => icon_rs::UNDO,
            IconId::Redo => icon_rs::REDO,
            IconId::AutoLayout => AUTO_LAYOUT,
            IconId::Comment => icon_rs::COMMENTS,
            IconId::Fit => FIT,
            IconId::Copy => icon_rs::COPY,
            IconId::OpenExternal => icon_rs::OPEN_EXTERNAL,
            IconId::Curve => icon_rs::SPLINE,
            IconId::Close => icon_rs::CLOSE,
            IconId::Consts => icon_rs::CUSTOM_TABLE,
            IconId::Check => icon_rs::CHECK,
            IconId::Plus => icon_rs::PLUS,
            IconId::Info => icon_rs::INFO,
            IconId::Link => icon_rs::LINK,
            IconId::Help => icon_rs::HELP,
        }
    }

    /// The icon of an asset of `kind` (`None`: unreadable file).
    pub fn for_asset(kind: Option<AssetKind>) -> IconId {
        match kind {
            Some(AssetKind::Function) => IconId::Function,
            Some(AssetKind::Material(MaterialKind::Pbr3d(_))) => IconId::Pbr,
            Some(AssetKind::Material(MaterialKind::Unlit3d(_))) => IconId::Unlit,
            Some(AssetKind::Material(MaterialKind::Sprite2d(_))) => IconId::Sprite,
            None => IconId::File,
        }
    }

    /// The icon of asset `id`; `kind` is its graph kind when it is a loaded graph.
    pub fn for_id(id: &AssetId, kind: Option<AssetKind>) -> IconId {
        match id.ty {
            AssetType::Curve => IconId::Curve,
            AssetType::Consts => IconId::Consts,
            AssetType::Material | AssetType::Function => IconId::for_asset(kind),
        }
    }

    /// The icon of the asset `kind` creates.
    pub fn for_new(kind: NewKind) -> IconId {
        match kind {
            NewKind::Pbr3d => IconId::Pbr,
            NewKind::Unlit3d => IconId::Unlit,
            NewKind::Sprite2d => IconId::Sprite,
            NewKind::Function => IconId::Function,
            NewKind::Curve => IconId::Curve,
            NewKind::Consts => IconId::Consts,
        }
    }

    fn has_accent_layer(self) -> bool {
        let icon = self.source();
        icon.kind.is_tintable() && icon.has_accent()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
enum Layer {
    /// A tintable icon's ink, in white.
    #[default]
    Ink,
    /// A tintable icon's accent detail, in white.
    Accent,
    /// A solid or sketch icon in its own (dark palette) colours.
    Full,
}

/// An icon of `size` logical px; its image is filled in by `resolve_icons`. Spawn with [`icon`].
#[derive(Component, Clone, Default)]
#[require(Node, ImageNode)]
pub struct UiIcon {
    pub id: IconId,
    pub size: f32,
    accent: bool,
}

/// An icon tinted `color` (the ink of glyph and line icons; only the alpha of solid and sketch icons).
pub fn icon(id: IconId, size: f32, color: Color) -> impl Scene {
    let accent: Vec<Box<dyn Scene>> = if id.has_accent_layer() {
        let layer = UiIcon { id, size, accent: true };
        vec![Box::new(bsn! {
            Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100) }
            ImageNode { color: ACCENT }
            layer
            Pickable::IGNORE
        })]
    } else {
        Vec::new()
    };
    let base = UiIcon { id, size, accent: false };
    bsn! {
        Node { width: px(size), height: px(size), flex_shrink: 0.0 }
        ImageNode { color: color }
        base
        Pickable::IGNORE
        Children [ {accent} ]
    }
}

pub struct IconsPlugin;

impl Plugin for IconsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<IconCache>().add_systems(PostUpdate, resolve_icons.before(bevy::ui::UiSystems::Prepare));
    }
}

/// Rasters by (icon, layer, physical px), and the scale they were last resolved for.
#[derive(Resource, Default)]
struct IconCache {
    scale: f32,
    images: HashMap<(IconId, Layer, u32), Handle<Image>>,
}

/// Fills in the image of new icons, and of all icons when the window's scale factor or `UiScale` changes.
fn resolve_icons(
    mut cache: ResMut<IconCache>,
    mut images: ResMut<Assets<Image>>,
    window: Query<&Window, With<PrimaryWindow>>,
    ui_scale: Res<UiScale>,
    mut q: Query<(Ref<UiIcon>, &mut ImageNode)>,
) {
    let scale = window.single().map_or(1.0, |w| w.scale_factor()) * ui_scale.0;
    let rescaled = scale != cache.scale;
    cache.scale = scale;
    for (icon, mut image) in &mut q {
        if !icon.is_changed() && !rescaled {
            continue;
        }
        let layer = match (icon.id.source().kind.is_tintable(), icon.accent) {
            (false, _) => Layer::Full,
            (true, false) => Layer::Ink,
            (true, true) => Layer::Accent,
        };
        let px = (icon.size * scale).round().clamp(1.0, 256.0) as u32;
        image.image = cache
            .images
            .entry((icon.id, layer, px))
            .or_insert_with(|| images.add(rasterize(&layer_svg(icon.id.source(), layer), px)))
            .clone();
        if layer == Layer::Full {
            image.color = Color::WHITE.with_alpha(image.color.alpha());
        }
    }
}

fn layer_svg(icon: Icon, layer: Layer) -> String {
    let (ink, accent) = (Palette::LIGHT.ink.to_hex(), Palette::LIGHT.accent.to_hex());
    let paint = |s: &str, ink_to: &str, accent_to: &str| {
        s.replace(&format!("=\"{ink}\""), &format!("=\"{ink_to}\""))
            .replace(&format!("=\"{accent}\""), &format!("=\"{accent_to}\""))
    };
    match layer {
        Layer::Ink => paint(icon.svg, "#ffffff", "none"),
        Layer::Accent => paint(icon.svg, "none", "#ffffff"),
        Layer::Full => icon.recolor(&ICON_PALETTE),
    }
}

/// `svg` rendered at `px` x `px` as a straight-alpha sRGB image. The SVGs are compiled in, so a parse error is a bug
/// (caught by `tests::every_icon_rasterizes`).
fn rasterize(svg: &str, px: u32) -> Image {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).expect("icon SVG parses");
    let mut pixmap = tiny_skia::Pixmap::new(px, px).expect("non-empty icon size");
    let s = px as f32 / tree.size().width().max(tree.size().height());
    resvg::render(&tree, tiny_skia::Transform::from_scale(s, s), &mut pixmap.as_mut());
    // tiny-skia stores premultiplied alpha, Bevy expects straight alpha.
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Image::new(
        Extent3d { width: px, height: px, depth_or_array_layers: 1 },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_rasterizes() {
        for id in IconId::ALL {
            let layers: &[Layer] = match (id.source().kind.is_tintable(), id.has_accent_layer()) {
                (false, _) => &[Layer::Full],
                (true, false) => &[Layer::Ink],
                (true, true) => &[Layer::Ink, Layer::Accent],
            };
            for &layer in layers {
                let image = rasterize(&layer_svg(id.source(), layer), 32);
                assert_eq!((image.width(), image.height()), (32, 32));
                let drawn = image.data.as_ref().unwrap().chunks(4).filter(|p| p[3] > 0).count();
                assert!(drawn > 20, "{id:?} {layer:?} is (nearly) empty");
            }
        }
    }

    #[test]
    fn tintable_layers_are_white() {
        for id in IconId::ALL.into_iter().filter(|id| id.source().kind.is_tintable()) {
            let svg = layer_svg(id.source(), Layer::Ink);
            assert!(!svg.contains(&Palette::LIGHT.ink.to_hex()), "{id:?}");
            assert!(!svg.contains(&Palette::LIGHT.accent.to_hex()), "{id:?}");
        }
    }

    #[test]
    fn custom_icons_use_only_palette_colours() {
        let p = Palette::LIGHT;
        let slots = [p.ink, p.top, p.soft, p.mid, p.shade, p.accent];
        for icon in [FOLDER_OPEN, MATERIAL_UNLIT, SAVE, SAVE_ALL, FIT, AUTO_LAYOUT] {
            let mut rest = icon.svg;
            while let Some(at) = rest.find("=\"#") {
                let hex = icon_rs::Rgb::from_hex(&rest[at + 2..at + 9]).unwrap();
                assert!(slots.contains(&hex), "{} uses {hex:?}", icon.name);
                rest = &rest[at + 9..];
            }
        }
    }
}
