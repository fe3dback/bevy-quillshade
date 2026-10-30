//! UI icons (decision 0013): icon-rs SVGs plus a few of our own (`icons/*.svg`), rasterized with resvg at the
//! icon's size in physical pixels, so they stay sharp at any window scale factor.
//!
//! The set is open: [`IconRef`] is a handle to any `&'static Icon`, so a domain declares its own icons as
//! `pub const X: IconRef = IconRef::new(&icon_rs::Y)` (or [`custom`]) next to its code. [`IconId`] holds only the core
//! (UI chrome) icons; every name must be unique across all sets (the rasterizer cache is keyed by it).
//!
//! Glyph and line icons are rendered in white and tinted through `ImageNode::color`; their accent detail is a
//! separate child layer in [`ACCENT`]. Solid and sketch icons keep the colours of icon-rs's dark palette with our
//! accent ([`ICON_PALETTE`]), the tint only fades them through its alpha.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;
use icon_rs::Palette;
pub use icon_rs::{Icon, Kind};
use resvg::{tiny_skia, usvg};

use crate::theme::{ACCENT, ACCENT_RGB};

/// Colours of solid and sketch icons: icon-rs's dark palette with the UI accent.
const ICON_PALETTE: Palette =
    Palette { accent: icon_rs::Rgb(ACCENT_RGB[0], ACCENT_RGB[1], ACCENT_RGB[2]), ..Palette::DARK };

// Our own icons, drawn by hand in icon-rs's house style (see icons/README.md). `svg_dark` is unused: the dark
// variant is made with `Icon::recolor`, as icon-rs makes its own.
const FOLDER_OPEN: Icon = custom("folder-open", Kind::Line, include_str!("../../icons/folder-open.svg"));
const SAVE: Icon = custom("save", Kind::Line, include_str!("../../icons/save.svg"));
const SAVE_ALL: Icon = custom("save-all", Kind::Line, include_str!("../../icons/save-all.svg"));
const FIT: Icon = custom("fit", Kind::Line, include_str!("../../icons/fit.svg"));
const AUTO_LAYOUT: Icon = custom("auto-layout", Kind::Line, include_str!("../../icons/auto-layout.svg"));

/// An icon with no dark variant of its own (see [`Icon::recolor`]); `name` also serves as its title.
pub const fn custom(name: &'static str, kind: Kind, svg: &'static str) -> Icon {
    Icon { name, kind, title: name, svg, svg_dark: svg }
}

/// A handle to an icon of any set; equal when the names are.
#[derive(Clone, Copy)]
pub struct IconRef(&'static Icon);

impl IconRef {
    pub const fn new(icon: &'static Icon) -> IconRef {
        IconRef(icon)
    }

    pub fn name(self) -> &'static str {
        self.0.name
    }

    pub fn icon(self) -> &'static Icon {
        self.0
    }

    fn has_accent_layer(self) -> bool {
        self.0.kind.is_tintable() && self.0.has_accent()
    }
}

impl PartialEq for IconRef {
    fn eq(&self, other: &Self) -> bool {
        self.name() == other.name()
    }
}

impl Eq for IconRef {}

impl Hash for IconRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name().hash(state);
    }
}

impl std::fmt::Debug for IconRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IconRef({})", self.name())
    }
}

impl Default for IconRef {
    fn default() -> Self {
        IconId::File.into()
    }
}

/// The core (UI chrome) icons.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum IconId {
    #[default]
    File,
    Folder,
    FolderOpen,
    CaretRight,
    CaretDown,
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
    Close,
    Check,
    Plus,
    Info,
    Link,
    Help,
    Warning,
    Image,
    ChevronDown,
    ChevronUp,
    /// Empties something (the gradients editor topbar's Clear).
    Clear,
    ArrowUp,
    ArrowDown,
    /// Navigation Back / Forward (the secondbar, View menu; decision 0041).
    Back,
    Forward,
    /// The project domain (design §14.2).
    Settings,
}

impl IconId {
    pub const ALL: [IconId; 36] = [
        IconId::File,
        IconId::Folder,
        IconId::FolderOpen,
        IconId::CaretRight,
        IconId::CaretDown,
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
        IconId::Close,
        IconId::Check,
        IconId::Plus,
        IconId::Info,
        IconId::Link,
        IconId::Help,
        IconId::Warning,
        IconId::Image,
        IconId::ChevronDown,
        IconId::ChevronUp,
        IconId::Clear,
        IconId::ArrowUp,
        IconId::ArrowDown,
        IconId::Back,
        IconId::Forward,
        IconId::Settings,
    ];

    fn source(self) -> &'static Icon {
        match self {
            IconId::File => &icon_rs::FILE,
            IconId::Folder => &icon_rs::FOLDER,
            IconId::FolderOpen => &FOLDER_OPEN,
            IconId::CaretRight => &icon_rs::CARET_RIGHT_FILLED,
            IconId::CaretDown => &icon_rs::CARET_DOWN_FILLED,
            IconId::FolderNew => &icon_rs::FOLDER_NEW,
            IconId::Edit => &icon_rs::EDIT,
            IconId::MoveToFolder => &icon_rs::MOVE_TO_FOLDER,
            IconId::Delete => &icon_rs::DELETE,
            IconId::Code => &icon_rs::CODE,
            IconId::Save => &SAVE,
            IconId::SaveAll => &SAVE_ALL,
            IconId::Regenerate => &icon_rs::SYNC_DOCUMENT,
            IconId::Undo => &icon_rs::UNDO,
            IconId::Redo => &icon_rs::REDO,
            IconId::AutoLayout => &AUTO_LAYOUT,
            IconId::Comment => &icon_rs::COMMENTS,
            IconId::Fit => &FIT,
            IconId::Copy => &icon_rs::COPY,
            IconId::OpenExternal => &icon_rs::OPEN_EXTERNAL,
            IconId::Close => &icon_rs::CLOSE,
            IconId::Check => &icon_rs::CHECK,
            IconId::Plus => &icon_rs::PLUS,
            IconId::Info => &icon_rs::INFO,
            IconId::Link => &icon_rs::LINK,
            IconId::Help => &icon_rs::HELP,
            IconId::Warning => &icon_rs::WARNING_FILLED,
            IconId::Image => &icon_rs::IMAGE,
            IconId::ChevronDown => &icon_rs::CHEVRON_DOWN,
            IconId::ChevronUp => &icon_rs::CHEVRON_UP,
            IconId::Clear => &icon_rs::REMOVE_CIRCLE,
            IconId::ArrowUp => &icon_rs::ARROW_UP,
            IconId::ArrowDown => &icon_rs::ARROW_DOWN,
            IconId::Back => &icon_rs::CHEVRON_LEFT,
            IconId::Forward => &icon_rs::CHEVRON_RIGHT,
            IconId::Settings => &icon_rs::SETTINGS,
        }
    }
}

impl From<IconId> for IconRef {
    fn from(id: IconId) -> IconRef {
        IconRef(id.source())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Layer {
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
    pub id: IconRef,
    pub size: f32,
    accent: bool,
}

/// An icon tinted `color` (the ink of glyph and line icons; only the alpha of solid and sketch icons). The accent
/// detail fades with `color`'s alpha too (a disabled button's dimmed icon).
pub fn icon(id: impl Into<IconRef>, size: f32, color: Color) -> impl Scene {
    let id = id.into();
    let accent: Vec<Box<dyn Scene>> = if id.has_accent_layer() {
        let (layer, tint) = (UiIcon { id, size, accent: true }, ACCENT.with_alpha(color.alpha()));
        vec![Box::new(bsn! {
            Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100) }
            ImageNode { color: tint }
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
    images: HashMap<(&'static str, Layer, u32), Handle<Image>>,
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
        let layer = match (icon.id.icon().kind.is_tintable(), icon.accent) {
            (false, _) => Layer::Full,
            (true, false) => Layer::Ink,
            (true, true) => Layer::Accent,
        };
        let px = (icon.size * scale).round().clamp(1.0, 256.0) as u32;
        image.image = cache
            .images
            .entry((icon.id.name(), layer, px))
            .or_insert_with(|| images.add(rasterize(&layer_svg(icon.id.icon(), layer), px)))
            .clone();
        if layer == Layer::Full {
            image.color = Color::WHITE.with_alpha(image.color.alpha());
        }
    }
}

pub fn layer_svg(icon: &Icon, layer: Layer) -> String {
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
/// (caught by `tests::core_icons_rasterize`).
pub fn rasterize(svg: &str, px: u32) -> Image {
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

/// Test helper for a domain's icon set: every `=\"#rrggbb\"` colour of the SVGs is a `Palette::LIGHT` slot.
pub fn assert_only_palette_colours(icons: &[Icon]) {
    let p = Palette::LIGHT;
    let slots = [p.ink, p.top, p.soft, p.mid, p.shade, p.accent];
    for icon in icons {
        let mut rest = icon.svg;
        while let Some(at) = rest.find("=\"#") {
            let hex = icon_rs::Rgb::from_hex(&rest[at + 2..at + 9]).unwrap();
            assert!(slots.contains(&hex), "{} uses {hex:?}", icon.name);
            rest = &rest[at + 9..];
        }
    }
}

/// Test helper for a domain's icon set: every icon rasterizes (all its layers) to a non-empty image.
pub fn assert_icons_rasterize(icons: &[IconRef]) {
    for &icon in icons {
        let layers: &[Layer] = match (icon.icon().kind.is_tintable(), icon.has_accent_layer()) {
            (false, _) => &[Layer::Full],
            (true, false) => &[Layer::Ink],
            (true, true) => &[Layer::Ink, Layer::Accent],
        };
        for &layer in layers {
            let image = rasterize(&layer_svg(icon.icon(), layer), 32);
            assert_eq!((image.width(), image.height()), (32, 32));
            let drawn = image.data.as_ref().unwrap().chunks(4).filter(|p| p[3] > 0).count();
            assert!(drawn > 20, "{icon:?} {layer:?} is (nearly) empty");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_icons_rasterize() {
        assert_icons_rasterize(&IconId::ALL.map(IconRef::from));
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
    fn core_icon_names_are_unique() {
        let mut names: Vec<_> = IconId::ALL.into_iter().map(|id| IconRef::from(id).name()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), IconId::ALL.len());
    }

    #[test]
    fn custom_icons_use_only_palette_colours() {
        assert_only_palette_colours(&[FOLDER_OPEN, SAVE, SAVE_ALL, FIT, AUTO_LAYOUT]);
    }

    #[test]
    fn icon_ref_eq_by_name() {
        assert_eq!(IconRef::from(IconId::Save), IconRef::new(&SAVE));
        assert_ne!(IconRef::from(IconId::Save), IconRef::from(IconId::SaveAll));
        let copy = IconRef::new(&const { custom("save", Kind::Line, "") });
        assert_eq!(IconRef::from(IconId::Save), copy);
        let set: std::collections::HashSet<_> = [IconRef::from(IconId::Save), copy].into();
        assert_eq!(set.len(), 1);
    }
}
