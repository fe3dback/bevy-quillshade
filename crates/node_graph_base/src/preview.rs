//! The in-node preview mechanism: generic read-only preview rows inside canvas nodes (a curve plot, a constant's
//! value, a texture thumbnail…). A node whose definition [`NodePreview::applies`] gets one row per preview kind,
//! between its title and its pins, or a column left of its output pins ([`NodePreview::BESIDE_OUTPUTS`]); the
//! domain's [`crate::GraphDomain::previews`] lists them ([`preview_row`]).
//!
//! The mechanism owns the row (its root entity, tagged [`PreviewRoot`], and a shared "n/a" child), resolves each
//! row's [`NodePreview::Key`] from the model (an open tab's unsaved data first, then disk) whenever the editor
//! changes, and calls [`NodePreview::draw`] only when the key (or, for [`NodePreview::SIZED`] kinds, the row's size)
//! changed. The row shows "n/a" while its key is `None` or the last `draw` reported a failure (`false`: e.g. an
//! image that does not decode); a failed draw is retried only when the key or size changes. It never touches the
//! canvas, so following an edit in another tab never triggers a canvas rebuild. What a row last drew is cached per
//! row entity and forgotten when the canvas rebuild despawns it; resources the content owns (images) live in its
//! components and go with it.

use std::collections::HashMap;
use std::marker::PhantomData;

use bevy::ecs::system::{StaticSystemParam, SystemParam, SystemParamItem};
use bevy::prelude::*;
use editor_core::doc_kind::TabDoc;
use editor_core::state::editor::Editor;
use editor_core::theme::TEXT_DIM;

use crate::view::PreviewRow;

/// A node id as the preview calls name it.
pub type NodeId = String;

/// One kind of in-node preview.
pub trait NodePreview: Send + Sync + 'static {
    /// The graph document the previewed nodes are in.
    type Doc: TabDoc;
    /// What decides whether a node gets the preview (a domain's node definition).
    type Def: ?Sized;
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
    /// Does a node of this definition get this preview?
    fn applies(def: &Self::Def) -> bool;
    /// Layout of the row's root; the content and then the "n/a" label are its children.
    fn layout(def: &Self::Def) -> Node;
    /// The row's content (spawned once with the node; [`NodePreview::draw`] updates it).
    fn content(def: &Self::Def) -> Box<dyn Scene>;
    /// Resolve the node's current key (open tab first, then disk); `None` shows "n/a".
    fn key(editor: &Editor, doc: &Self::Doc, node: &NodeId) -> Option<Self::Key>;
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

/// The row of preview kind `P` of model node `node`, if `P` applies to `def`.
pub fn preview_row<P: NodePreview>(def: &P::Def, node: &NodeId) -> Option<Box<dyn Scene>> {
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

/// [`preview_row`] placed as its kind asks ([`NodePreview::BESIDE_OUTPUTS`]).
pub fn preview<P: NodePreview>(def: &P::Def, node: &NodeId) -> Option<PreviewRow> {
    preview_row::<P>(def, node).map(|scene| PreviewRow { scene, beside_outputs: P::BESIDE_OUTPUTS })
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
    let Some(doc) = editor.active_doc::<P::Doc>() else { return };
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
