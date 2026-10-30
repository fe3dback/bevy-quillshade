//! The node graph UI shared by every graph domain (design §7.3): the bevy_noodle canvas of the active graph and its
//! gestures, the palette shell, the in-node preview mechanism, the minimap, the graph editor topbar tools, the base
//! input context, the comment frame and multi-selection subpanes, the grid and the auto-layout. The graph's model
//! stays in the domain, behind [`GraphDomain`]; [`register_graph`] adds a domain's graph editor kind.
//!
//! Depends on `editor_core` (and Bevy / bevy_noodle) only: never on a domain or a `shader_*` crate.

#![allow(clippy::type_complexity)] // Bevy system signatures

use bevy::prelude::*;
use bevy_noodle::prelude::*;
use editor_core::domain::DomainRegistrar;

pub mod canvas;
pub mod domain;
pub mod edit;
pub mod editor;
pub mod input;
pub mod layout;
pub mod palette;
pub mod preview;
pub mod view;

pub use canvas::{
    CanvasPlugin, CanvasSystems, EditorCanvas, EditorCanvasMarker, FrameResizeHandle, GraphOf, GraphViews,
};
pub use domain::{GraphCtxRef, GraphDomain, active};
pub use edit::{GraphEdit, PinKey, PinRef, graph_edit};
pub use editor::{GraphCtx, GraphEditor};
pub use layout::{EMPTY_FRAME_SIZE, GRID_SNAP, MIN_FRAME_SIZE, snap, tidy};
pub use view::{
    DiagMark, DropItem, Emphasis, FrameField, FrameView, GraphHotkey, GraphView, MovePlan, NodeLook, NodeShape,
    NodeView, PaletteEntry, PickFn, PinLook, PlaceSpec, PreviewRow,
};

/// Adds the graph editor kind of `G` (design §7.3): its editor topbar, right sidebar and input context
/// ([`GraphEditor`]), the canvas view and its minimap, the canvas sync (also headless) and the window's gestures
/// and palette.
pub fn register_graph<G: GraphDomain>(r: &mut DomainRegistrar) {
    r.editor_kind::<GraphEditor<G>>()
        .logic(graph_logic::<G>)
        .bevy(graph_window::<G>)
        .doc_view(|| Box::new(canvas::graph_view::<G>()))
        .doc_overlay(|| Box::new(input::minimap::<G>()));
}

/// The canvas sync of `G` (`DomainRegistrar::logic`, also headless): the bevy_noodle plugins unless the app has
/// them (UI tests add the core one themselves), `G`'s view in [`canvas::GraphViews`], the canvas attach and its sync
/// after the actions are applied.
fn graph_logic<G: GraphDomain>(app: &mut App) {
    if !app.is_plugin_added::<NoodleCorePlugin>() {
        app.add_plugins((NoodlePlugins, NoodleDefaultStylePlugin));
    }
    // Once for every graph domain: the hook is about canvas nodes, not one domain's graph.
    if !app.is_plugin_added::<InlineEditsPlugin>() {
        app.add_plugins(InlineEditsPlugin);
    }
    app.init_resource::<canvas::GraphViews>().world_mut().resource_mut::<canvas::GraphViews>().0.push(G::VIEW);
    app.add_plugins(CanvasPlugin::<G>::default())
        .configure_sets(Update, CanvasSystems.after(editor_core::apply_actions))
        .add_systems(PreUpdate, canvas::attach_canvas::<G>);
}

/// Inline editors on canvas nodes (design §7.3): the last edit of a property field inside a node (core's
/// [`editor_core::ui::props::FieldEditEnded`]) ends its undo step with [`editor_core::action::TabAction::CommitInline`],
/// and the right sidebar catches up.
pub struct InlineEditsPlugin;

impl Plugin for InlineEditsPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(commit_inline);
    }
}

fn commit_inline(
    ev: On<editor_core::ui::props::FieldEditEnded>,
    graph: GraphQuery,
    mut actions: ResMut<editor_core::action::Actions>,
) {
    if graph.node_of(ev.source).is_some() {
        actions.push(editor_core::action::Action::Tab(editor_core::action::TabAction::CommitInline));
    }
}

/// The window parts of `G` (`DomainRegistrar::bevy`): the canvas gestures and the palette.
fn graph_window<G: GraphDomain>(app: &mut App) {
    app.add_plugins((input::InputPlugin::<G>::default(), palette::PalettePlugin::<G>::default()));
}
