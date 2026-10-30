//! Thumbnail of the texture a texture node uses (design §4.5), under the title of `TextureSample`,
//! `TextureObject`, `param::TextureSample2D` and `param::TextureObject` (any node with a `ParamKind::Texture`
//! parameter): the image decoded with the Texture asset's settings, fitted into a 128×128 square left of the output pins (aspect kept,
//! letterboxed), the built-in default texture when the node has none, "n/a" for an unknown Texture asset or an image
//! that does not load.
//!
//! The key is what changes the picture — the image key, the settings and the file's stamp — resolved through
//! [`Editor::texture_image`] (an open Texture tab's unsaved path and settings first, then disk), so the thumbnail
//! follows edits in a Texture tab without canvas rebuilds. Like every preview it is re-keyed when the editor changes:
//! a file replaced on disk shows after the next edit or canvas rebuild (tab switch). Images come from [`GameImageCache`] (shared with the
//! Texture tab's preview, one entry per image and settings); a row holds its handle while it shows it, so the cache
//! frees the image once no row or tab shows it. A `TextureSample` whose `tex` input is wired straight to a
//! TextureObject or texture parameter shows that node's texture (the wire overrides the property); wired to anything
//! else (reroute, function input, …) it shows its own property's texture.

use crate::material_state::MaterialStates;
use bevy::ecs::system::SystemParamItem;
use bevy::prelude::*;
use shader_graph_core::Value;
use shader_graph_core::model::NodeId;
use shader_graph_core::registry::{NodeDef, ParamKind};
use shader_graph_core::texture::{INTERNAL_PREFIX, MATERIAL_TEXTURES, TextureAsset, TextureSettings};
use shader_stdlib::internal::DEFAULT_TEXTURE;

use super::node_preview::{NodePreview, PreviewTarget};
use crate::doc::Doc;
use crate::ops::Editor;
use crate::ops::prelude::*;
use crate::texture::{FileStamp, GameImageCache};

/// Side of the thumbnail square (logical px).
pub const SIDE: f32 = 128.0;
/// Texture input of sampling nodes; a wired texture overrides the property.
const TEX_PIN: &str = "tex";
/// Background of the letterbox bars and of transparent pixels.
const LETTERBOX: Color = Color::srgb_u8(24, 25, 29);

/// What a texture thumbnail shows.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureKey {
    /// What [`GameImageCache::get`] decodes: a path under `assets/` or an `@internal/…` id.
    pub image: String,
    pub settings: TextureSettings,
    /// The file's stamp when the key was taken (`None`: built-in or missing), so a changed file is decoded again.
    stamp: Option<FileStamp>,
}

/// Part of a thumbnail row.
#[derive(Component, Clone, Copy, Default, PartialEq)]
pub enum TexturePart {
    /// The thumbnail square (letterbox background).
    #[default]
    Frame,
    /// The image, sized to fit the square.
    Image,
}

/// Thumbnail of nodes with a Texture asset property.
pub struct TexturePreview;

impl NodePreview for TexturePreview {
    type Doc = crate::doc::Doc;
    type Def = dyn shader_graph_core::registry::NodeDef;
    type Key = TextureKey;
    type Draw = (
        Res<'static, Editor>,
        ResMut<'static, GameImageCache>,
        ResMut<'static, Assets<Image>>,
        Query<'static, 'static, &'static Children>,
        Query<'static, 'static, (&'static TexturePart, &'static mut Node, Option<&'static mut ImageNode>)>,
    );

    fn applies(def: &dyn NodeDef) -> bool {
        texture_param(def).is_some()
    }

    const BESIDE_OUTPUTS: bool = true;

    fn layout(_: &dyn NodeDef) -> Node {
        // A fixed size: the node keeps its size whether the thumbnail or "n/a" shows.
        Node {
            width: px(SIDE),
            height: px(SIDE),
            flex_shrink: 0.0,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        }
    }

    fn content(_: &dyn NodeDef) -> Box<dyn Scene> {
        // Out of the row's flow (the hidden "n/a" label still takes room there), centred over the whole row.
        Box::new(bsn! {
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                top: px(0),
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
            }
            Pickable::IGNORE
            Children [
                TexturePart::Frame
                Node {
                    display: Display::None,
                    width: px(SIDE),
                    height: px(SIDE),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::all(px(3)),
                    overflow: Overflow::clip(),
                }
                BackgroundColor(LETTERBOX)
                Pickable::IGNORE
                Children [
                    TexturePart::Image
                    Node { width: px(SIDE), height: px(SIDE) }
                    ImageNode { color: Color::NONE }
                    Pickable::IGNORE
                ]
            ]
        })
    }

    fn key(editor: &Editor, doc: &Doc, node: &NodeId) -> Option<TextureKey> {
        let n = doc.asset.nodes.get(node)?;
        // A texture wired into `tex` wins over the property: one hop to a node naming its texture (TextureObject,
        // a texture parameter); any other source (reroute, function input, …) leaves the property's texture.
        let wired = doc.asset.edge_into(node, TEX_PIN).and_then(|e| doc.asset.nodes.get(&e.from.0));
        let n = match wired {
            Some(src)
                if editor
                    .domains
                    .material()
                    .registry
                    .get(&src.kind)
                    .is_some_and(|d| texture_param(d.as_ref()).is_some()) =>
            {
                src
            }
            _ => n,
        };
        let param = texture_param(editor.domains.material().registry.get(&n.kind)?.as_ref())?;
        let params = editor.domains.material().registry.resolved_params(&n.kind, &n.params);
        let id = params.get(&param).map(Value::as_str).filter(|id| !id.is_empty()).unwrap_or(DEFAULT_TEXTURE);
        // Unsaved path and settings of an open Texture tab first; `None` for an unknown Texture asset.
        let (image, settings) = editor.texture_image(id)?;
        // Only paths the cache would read are stat-ed (nothing outside `assets/`).
        let file = !image.starts_with(INTERNAL_PREFIX) && TextureAsset::check_path(&image, &MATERIAL_TEXTURES).is_ok();
        let stamp = match (&editor.project, file) {
            (Some(project), true) => FileStamp::of(&project.assets_dir().join(&image)),
            _ => None,
        };
        Some(TextureKey { image, settings, stamp })
    }

    fn draw(
        key: Option<&TextureKey>,
        at: PreviewTarget,
        (editor, cache, images, children, parts): &mut SystemParamItem<Self::Draw>,
    ) -> bool {
        let handle = match (key, &editor.project) {
            (Some(k), Some(project)) => cache.get(project, &k.image, &k.settings, images).ok(),
            _ => None,
        };
        let fitted = handle.as_ref().and_then(|h| images.get(h)).map(|image| fit(image.size()));
        let shown = fitted.is_some();
        for e in children.iter_descendants(at.root) {
            let Ok((part, mut node, image)) = parts.get_mut(e) else { continue };
            match part {
                TexturePart::Frame => {
                    let display = if shown { Display::Flex } else { Display::None };
                    if node.display != display {
                        node.display = display;
                    }
                }
                TexturePart::Image => {
                    let Some(mut image) = image else { continue };
                    match (&handle, fitted) {
                        (Some(h), Some(size)) => {
                            let (w, hgt) = (px(size.x), px(size.y));
                            if node.width != w || node.height != hgt {
                                node.width = w;
                                node.height = hgt;
                            }
                            image.image = h.clone();
                            image.color = Color::WHITE;
                        }
                        // Drop the handle: the cache frees an image nobody shows.
                        _ => *image = ImageNode { color: Color::NONE, ..default() },
                    }
                }
            }
        }
        shown
    }
}

/// Size (logical px) of an image of `size` px fitted into the square: aspect kept, the longer side spans it (small
/// images are enlarged), whole px, at least 1.
fn fit(size: UVec2) -> Vec2 {
    let size = size.as_vec2().max(Vec2::ONE);
    (size * (SIDE / size.max_element())).round().max(Vec2::ONE)
}

/// Id of the Texture asset property of nodes of this definition (`texture` on stdlib nodes, `default` on parameter
/// nodes); `None`: not a texture node.
fn texture_param(def: &dyn NodeDef) -> Option<String> {
    def.params().into_iter().find(|p| p.kind == ParamKind::Texture).map(|p| p.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{Action, GraphProp, TextureAction, TextureEdit, set_value};
    use shader_project::{AssetId, AssetType, Project, fixtures};

    #[test]
    fn fit_keeps_aspect() {
        assert_eq!(fit(UVec2::new(128, 128)), Vec2::splat(SIDE));
        assert_eq!(fit(UVec2::new(16, 16)), Vec2::splat(SIDE), "enlarged");
        assert_eq!(fit(UVec2::new(256, 64)), Vec2::new(SIDE, SIDE / 4.0), "letterboxed");
        assert_eq!(fit(UVec2::new(10, 300)), Vec2::new((10.0 * SIDE / 300.0).round(), SIDE), "pillarboxed");
        assert_eq!(fit(UVec2::new(4096, 1)), Vec2::new(SIDE, 1.0), "at least 1 px");
    }

    fn key(e: &Editor, node: &NodeId) -> Option<TextureKey> {
        TexturePreview::key(e, e.doc()?, node)
    }

    fn editor() -> (tempfile::TempDir, Editor) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
        std::fs::create_dir_all(dir.path().join("assets")).unwrap();
        fixtures::save_all(&Project::open(dir.path()).unwrap()).unwrap();
        let mut e = crate::test_editor();
        e.open_project(dir.path()).unwrap();
        (dir, e)
    }

    #[test]
    fn texture_preview_follows_the_tex_wire() {
        let (_dir, mut e) = editor();
        e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/water").to_ref()));
        let samples: Vec<NodeId> = {
            let d = e.doc().unwrap();
            d.asset.nodes.iter().filter(|(_, n)| n.kind == "stdlib::TextureSample").map(|(k, _)| k.clone()).collect()
        };
        assert_eq!(samples.len(), 2);
        for s in &samples {
            assert_eq!(key(&e, s).unwrap().image, "textures/water_normal.ktx2", "{s}: the wired TextureObject");
        }
        e.apply(crate::ops::GraphEdit::Disconnect { to: (samples[0].clone(), TEX_PIN.into()) });
        assert_eq!(key(&e, &samples[0]).unwrap().image, DEFAULT_TEXTURE, "unwired: the empty property");
    }

    #[test]
    fn texture_preview_follows_open_tab() {
        let (_dir, mut e) = editor();
        e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
        let graph = e.active.unwrap();
        let node = {
            let d = e.doc().unwrap();
            let albedo = Value::Str("textures/rock_albedo".into());
            d.asset.nodes.iter().find(|(_, n)| n.params.get("default") == Some(&albedo)).unwrap().0.clone()
        };
        let revision = e.doc().unwrap().revision;

        let k = key(&e, &node).unwrap();
        assert_eq!((k.image.as_str(), &k.settings), ("textures/rock_albedo.ktx2", &TextureSettings::default()));

        // Unsaved edits of the open Texture tab show on the node; the graph document is untouched (no rebuild).
        let albedo = AssetId::new(AssetType::Texture, "textures/rock_albedo");
        e.apply(Action::OpenAsset(albedo.to_ref()));
        e.apply(Action::from(TextureEdit::new(TextureAction::SetPath("textures/rock_albedo.png".into()))));
        let linear = TextureSettings { is_srgb: false, ..Default::default() };
        e.apply(Action::from(TextureEdit::new(TextureAction::SetSettings(linear.clone()))));
        e.apply(Action::ActivateTab(graph));
        let k = key(&e, &node).unwrap();
        assert_eq!((k.image.as_str(), &k.settings), ("textures/rock_albedo.png", &linear));
        assert_eq!(e.doc().unwrap().revision, revision, "graph document unchanged");

        // No texture: the built-in default; an unknown Texture asset: n/a.
        let target = GraphProp::Param { node: node.clone(), key: "default".into() };
        e.apply(set_value(target.clone(), Value::Str(String::new()), false));
        assert_eq!(key(&e, &node).unwrap().image, DEFAULT_TEXTURE);
        e.apply(set_value(target, Value::Str("textures/missing".into()), false));
        assert!(key(&e, &node).is_none(), "n/a for an unknown Texture asset");
    }
}
