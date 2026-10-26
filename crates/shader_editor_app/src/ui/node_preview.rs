//! Generic read-only preview rows inside canvas nodes (curve plot of a Sample Curve node, value of a `Global*`
//! node's constant, thumbnail of a texture node's Texture asset): a node whose definition [`NodePreview::applies`]
//! gets one row per preview kind, between its title and its pins, or a column left of its output pins
//! ([`NodePreview::BESIDE_OUTPUTS`]).
//!
//! The mechanism owns the row (its root entity, tagged [`PreviewRoot`], and a shared "n/a" child), resolves each
//! row's [`NodePreview::Key`] from the model (an open tab's unsaved data first, then disk) whenever the editor
//! changes, and calls [`NodePreview::draw`] only when the key (or, for [`NodePreview::SIZED`] kinds, the row's size)
//! changed. The row shows "n/a" while its key is `None` or the last `draw` reported a failure (`false`: e.g. an
//! image that does not decode); a failed draw is retried only when the key or size changes. It never touches the
//! canvas, so following an edit in another tab never triggers a canvas rebuild. What a
//! row last drew is cached per row entity and forgotten when the canvas rebuild despawns it; resources the content
//! owns (images) live in its components and go with it.
//!
//! A new preview kind: implement [`NodePreview`] and add it to `kinds()`.

use std::collections::HashMap;
use std::marker::PhantomData;

use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::{StaticSystemParam, SystemParam, SystemParamItem};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use resvg::tiny_skia;
use shader_graph_core::model::NodeId;
use shader_graph_core::registry::NodeDef;

use super::TEXT_DIM;
use super::const_preview::ConstPreview;
use super::curve_preview::CurvePreview;
use super::texture_preview::TexturePreview;
use crate::doc::Doc;
use crate::game_image::GameImageCache;
use crate::state::Editor;

/// One kind of in-node preview.
pub trait NodePreview: Send + Sync + 'static {
    /// Data the preview depends on; the preview is redrawn only when it changes.
    type Key: Clone + PartialEq + Send + Sync + 'static;
    /// What [`NodePreview::draw`] reads and writes (queries of the content's components, `Assets<Image>`, caches).
    /// It runs in the same system as the mechanism, so it must not write `Editor`, [`PreviewRoot`], `ComputedNode`
    /// or `Children` (the "n/a" label's `Visibility` is set through commands and does not conflict).
    type Draw: SystemParam + 'static;
    /// The content is rasterized at the row's size: it is drawn once the row is laid out and again on every resize.
    const SIZED: bool = false;
    /// The row is a column left of the node's output pins instead of a full-width row above them (when the node has
    /// outputs); its layout gives it a fixed width.
    const BESIDE_OUTPUTS: bool = false;
    /// Does a node of this definition get this preview (scan of `def.params()` for a `ParamKind`)?
    fn applies(def: &dyn NodeDef) -> bool;
    /// Layout of the row's root; the content and then the "n/a" label are its children.
    fn layout(def: &dyn NodeDef) -> Node;
    /// The row's content (spawned once with the node; [`NodePreview::draw`] updates it).
    fn content(def: &dyn NodeDef) -> Box<dyn Scene>;
    /// Resolve the node's current key (open tab first, then disk); `None` shows "n/a".
    fn key(editor: &Editor, doc: &Doc, node: &NodeId) -> Option<Self::Key>;
    /// Updates the content under `at.root` to show `key` (`None`: the row shows "n/a"; hide or clear the content).
    /// Returns whether the content shows `key`; `false` (the data does not draw, e.g. an image fails to decode)
    /// shows "n/a" as well. The result is ignored for a `None` key.
    fn draw(key: Option<&Self::Key>, at: PreviewTarget, param: &mut SystemParamItem<Self::Draw>) -> bool;
}

/// The row a [`NodePreview::draw`] call updates.
#[derive(Clone, Copy, Debug)]
pub struct PreviewTarget {
    /// The row's root entity (the content is among its descendants).
    pub root: Entity,
    /// Size of the root in physical px (zero before the first layout of a non-[`NodePreview::SIZED`] kind).
    pub size: UVec2,
    /// Physical px per logical px.
    pub scale: f32,
}

/// Root of a preview row of kind `P`, showing model node `node`.
#[derive(Component)]
pub struct PreviewRoot<P: NodePreview> {
    pub node: NodeId,
    kind: PhantomData<fn() -> P>,
}

impl<P: NodePreview> PreviewRoot<P> {
    pub fn new(node: &str) -> Self {
        PreviewRoot { node: node.into(), kind: PhantomData }
    }
}

impl<P: NodePreview> Default for PreviewRoot<P> {
    fn default() -> Self {
        Self::new("")
    }
}

impl<P: NodePreview> Clone for PreviewRoot<P> {
    fn clone(&self) -> Self {
        Self::new(&self.node)
    }
}

/// The "n/a" label of a preview row (any kind), shown while the row has no key or its key failed to draw.
#[derive(Component, Default, Clone)]
pub struct PreviewNa;

/// Updates the rows of preview kind `P`.
pub struct NodePreviewPlugin<P: NodePreview>(PhantomData<fn() -> P>);

impl<P: NodePreview> Default for NodePreviewPlugin<P> {
    fn default() -> Self {
        NodePreviewPlugin(PhantomData)
    }
}

impl<P: NodePreview> Plugin for NodePreviewPlugin<P> {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, update_previews::<P>.before(bevy::ui::UiSystems::Prepare));
    }
}

/// Every preview kind: [`NodePreviewPlugin`] for each.
pub struct NodePreviewsPlugin;

impl Plugin for NodePreviewsPlugin {
    fn build(&self, app: &mut App) {
        // Texture thumbnails decode through the shared cache (`GameImagePlugin` frees its unused entries).
        app.init_resource::<GameImageCache>();
        for k in kinds() {
            (k.plugin)(app);
        }
    }
}

/// One registered preview kind, type-erased.
struct Kind {
    row: fn(&dyn NodeDef, &NodeId) -> Option<Box<dyn Scene>>,
    beside_outputs: bool,
    plugin: fn(&mut App),
}

impl Kind {
    fn of<P: NodePreview>() -> Self {
        Kind {
            row: preview_row::<P>,
            beside_outputs: P::BESIDE_OUTPUTS,
            plugin: |app| _ = app.add_plugins(NodePreviewPlugin::<P>::default()),
        }
    }
}

/// Every preview kind, in row order.
fn kinds() -> [Kind; 3] {
    [Kind::of::<CurvePreview>(), Kind::of::<ConstPreview>(), Kind::of::<TexturePreview>()]
}

/// The previews of model node `node` of definition `def` (used by the canvas when it builds the node).
#[derive(Default)]
pub struct Previews {
    /// Full-width rows under the title.
    pub rows: Vec<Box<dyn Scene>>,
    /// Columns left of the output pins ([`NodePreview::BESIDE_OUTPUTS`]).
    pub beside_outputs: Vec<Box<dyn Scene>>,
}

pub fn previews(def: &dyn NodeDef, node: &NodeId) -> Previews {
    let mut out = Previews::default();
    for k in kinds() {
        if let Some(row) = (k.row)(def, node) {
            if k.beside_outputs { &mut out.beside_outputs } else { &mut out.rows }.push(row);
        }
    }
    out
}

/// The row of preview kind `P` of model node `node`, if `P` applies to `def`.
pub fn preview_row<P: NodePreview>(def: &dyn NodeDef, node: &NodeId) -> Option<Box<dyn Scene>> {
    if !P::applies(def) {
        return None;
    }
    let (tag, layout, content) = (PreviewRoot::<P>::new(node), P::layout(def), P::content(def));
    Some(Box::new(bsn! {
        tag
        layout
        Pickable::IGNORE
        Children [
            @{content}
            --
            PreviewNa
            Text("n/a")
            TextFont { font_size: FontSize::Px(12.0) }
            TextColor(TEXT_DIM)
            Visibility::Hidden
            Pickable::IGNORE
        ]
    }))
}

/// What a row last drew.
struct Shown<K> {
    key: Option<K>,
    size: UVec2,
    /// "n/a" is visible.
    na: bool,
}

fn update_previews<P: NodePreview>(
    editor: Res<Editor>,
    roots: Query<(Entity, &PreviewRoot<P>, &ComputedNode, &Children)>,
    na: Query<(), With<PreviewNa>>,
    mut commands: Commands,
    mut param: StaticSystemParam<P::Draw>,
    mut shown: Local<HashMap<Entity, Shown<P::Key>>>,
) {
    // Canvas rebuilds respawn the nodes: forget the rows that are gone.
    shown.retain(|e, _| roots.contains(*e));
    // The canvas shows the active graph document; nothing to follow while another kind of tab is active.
    let Some(doc) = editor.doc() else { return };
    let changed = editor.is_changed();
    for (root, p, computed, children) in &roots {
        let size = computed.size().as_uvec2();
        if P::SIZED && (size.x == 0 || size.y == 0) {
            continue;
        }
        let size = if P::SIZED { size } else { UVec2::ZERO };
        let known = shown.get(&root);
        if known.is_some_and(|s| !changed && s.size == size) {
            continue;
        }
        let key = P::key(&editor, doc, &p.node);
        if known.is_some_and(|s| s.key == key && s.size == size) {
            continue;
        }
        let at = PreviewTarget { root, size: computed.size().as_uvec2(), scale: 1.0 / computed.inverse_scale_factor() };
        let drawn = P::draw(key.as_ref(), at, &mut param);
        let show_na = key.is_none() || !drawn;
        // A new row spawns with "n/a" hidden.
        if known.is_some_and(|s| s.na) != show_na {
            let vis = if show_na { Visibility::Inherited } else { Visibility::Hidden };
            for c in children.iter().filter(|c| na.contains(*c)) {
                commands.entity(c).insert(vis);
            }
        }
        shown.insert(root, Shown { key, size, na: show_na });
    }
}

/// A pixmap with an opaque background as an image: its premultiplied pixels equal the straight ones Bevy expects.
pub(super) fn opaque_image(pixmap: tiny_skia::Pixmap) -> Image {
    let (width, height) = (pixmap.width(), pixmap.height());
    Image::new(
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        pixmap.take(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

pub(super) fn skia_color(c: Color, alpha: f32) -> tiny_skia::Color {
    let c = c.to_srgba();
    tiny_skia::Color::from_rgba(c.red, c.green, c.blue, c.alpha * alpha).unwrap_or(tiny_skia::Color::BLACK)
}
