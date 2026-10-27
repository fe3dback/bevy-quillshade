//! Editor kinds (design §2.1, decision 0014): one module per kind of tab content — graph, text, curve, consts,
//! image, texture — holding every trait impl of that kind ([`crate::doc_kind::TabDoc`] today). The document types
//! themselves stay in their own files (`doc.rs`, `curve_doc.rs`, …); what the editor asks of a kind lives here.
//!
//! The window areas a kind owns (design §1, §2.1) are described by [`EditorAreas`], implemented by one zero-sized type
//! per kind ([`graph::GraphEditor`], …): the editor topbar and the right sidebar (decision 0031). Each area is spawned
//! once per kind inside its strip, tagged [`AreaOf`], and only the active kind's is displayed (`ui::areas`); the
//! kind's own systems ([`EditorAreas::build`]) fill it.
//!
//! Hotkeys: a kind with any implements [`EditorInput`] — its bevy_enhanced_input context, active while a tab of the
//! kind is ([`main_input`], decision 0032).
//!
//! Also the data of the editor statusbar (design §2.3): [`StatusInfo`], built by [`crate::doc_kind::TabDoc::status`]
//! from a [`StatusCtx`] the editor fills ([`crate::state::Editor::status_info`]).

use bevy::prelude::*;
use shader_graph_core::Diagnostic;

use crate::state::Editor;
use crate::tab::ViewKind;

pub mod consts;
pub mod curve;
pub mod graph;
pub mod image;
pub mod main_input;
pub mod text;
pub mod texture;

pub use consts::ConstsEditor;
pub use curve::{CurveCtx, CurveEditor};
pub use graph::{GraphCtx, GraphEditor};
pub use image::ImageEditor;
pub use text::{TextCtx, TextEditor};
pub use texture::TextureEditor;

/// The window areas an editor kind owns, besides its editor content (the view in `ui::DocArea`). Each one is a
/// persistent scene spawned once at startup inside its strip and filled by the kind's own redraw systems (keyed on
/// what they show, never every frame); `None` means the kind has no such area and the strip is hidden for it.
pub trait EditorAreas {
    /// The view of the tabs this kind edits ([`crate::doc_kind::TabDoc::view`]).
    const VIEW: ViewKind;
    /// Header of the right sidebar while this kind is active.
    const SIDEBAR_TITLE: &'static str;
    /// The editor topbar row (under the tabs bar).
    fn topbar() -> Option<impl Scene>;
    /// The right sidebar body (under the header showing [`Self::SIDEBAR_TITLE`]).
    fn sidebar() -> Option<impl Scene>;
    /// Registers the systems that fill this kind's areas.
    fn build(_app: &mut App) {}
    /// Registers the kind's input context and its handlers: [`main_input::add_kind`] for a kind with an
    /// [`EditorInput`]. Called by [`main_input::InputContextsPlugin`] (the UI side), not by [`EditorAreas::build`].
    fn input(_app: &mut App) {}
}

/// The hotkeys of an editor kind (design §2.2): a bevy_enhanced_input context on [`main_input::InputRoot`], active
/// only while the active tab is of this kind.
pub trait EditorInput: EditorAreas {
    /// The context component.
    type Ctx: Component + Default;
    /// `Actions<Self::Ctx> [ … ]`: the context's actions, built with [`main_input::hotkey`].
    fn bindings() -> impl Scene;
}

/// Calls [`KindVisitor::visit`] once per editor kind: the one list of kinds the window is built from.
pub trait KindVisitor {
    fn visit<E: EditorAreas>(&mut self);
}

pub fn for_each_kind(v: &mut impl KindVisitor) {
    v.visit::<GraphEditor>();
    v.visit::<TextEditor>();
    v.visit::<CurveEditor>();
    v.visit::<ConstsEditor>();
    v.visit::<ImageEditor>();
    v.visit::<TextureEditor>();
}

/// The row a kind's editor topbar is built on: a marker plus this, filled by the kind's redraw system.
pub fn bar_row() -> impl Scene {
    bsn! {
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(8),
            padding: UiRect::axes(px(10), px(2)),
        }
    }
}

/// The column a kind's right sidebar body is built on: a marker plus this, filled by the kind's redraw system through
/// [`SidebarKey`] and [`fill_sidebar`].
pub fn sidebar_body() -> impl Scene {
    bsn! {
        Node { flex_direction: FlexDirection::Column, row_gap: px(6) }
    }
}

/// When a kind's right sidebar body is rebuilt: on a change of `(Editor::ui.props, Editor::active)` while the kind is
/// active (the key the Properties panel always had). It is forgotten while another kind is active, so the body is
/// rebuilt when its kind comes back.
#[derive(Default)]
pub struct SidebarKey(Option<(u64, Option<usize>)>);

impl SidebarKey {
    /// The body to refill when the key changed since the last rebuild, `None` when nothing is to be done.
    pub fn changed<M: Component>(&mut self, editor: &Editor, body: &Query<Entity, With<M>>) -> Option<Entity> {
        let key = (editor.ui.props, editor.active);
        if self.0 == Some(key) {
            return None;
        }
        let body = body.single().ok()?;
        self.0 = Some(key);
        Some(body)
    }

    /// The kind is not active.
    pub fn forget(&mut self) {
        self.0 = None;
    }
}

/// Replaces the rows of a right sidebar body.
pub fn fill_sidebar(commands: &mut Commands, body: Entity, rows: Vec<Box<dyn Scene>>) {
    commands.entity(body).despawn_children();
    for r in rows {
        commands.spawn_scene(bsn! { @{r} ChildOf(body) });
    }
}

/// A per-kind area container: `Display::Flex` while the active tab is of this kind, `Display::None` otherwise (not
/// `Visibility`: hidden flow children must not take space).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct AreaOf(pub ViewKind);

impl Default for AreaOf {
    fn default() -> Self {
        AreaOf(ViewKind::Graph)
    }
}

/// What the editor knows about the active tab beyond its document: this asset's diagnostics and the line count of
/// its generated WESL.
#[derive(Debug, Clone, Copy, Default)]
pub struct StatusCtx<'a> {
    /// Diagnostics of the tab's asset only (live validation plus regeneration-only codes).
    pub diagnostics: &'a [Diagnostic],
    /// Lines of the asset's `.wesl` as last written by regeneration or read by a generated text tab.
    pub wesl_lines: Option<usize>,
}

/// The editor statusbar's content (design §2.3): only the important bits.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatusInfo {
    /// The document has unsaved changes.
    pub dirty: bool,
    pub errors: usize,
    pub warnings: usize,
    /// Kind-specific counts, shown in order (`12 nodes`, `340 WESL lines`).
    pub counts: Vec<String>,
}

impl StatusInfo {
    /// Errors / warnings counted from `ctx`.
    pub fn new(dirty: bool, ctx: &StatusCtx, counts: Vec<String>) -> StatusInfo {
        let errors = ctx.diagnostics.iter().filter(|d| d.is_error()).count();
        StatusInfo { dirty, errors, warnings: ctx.diagnostics.len() - errors, counts }
    }
}

#[cfg(test)]
mod tests {
    use shader_graph_core::Code;

    use super::*;

    #[test]
    fn status_counts_errors_and_warnings() {
        let diagnostics = [
            Diagnostic::new(Code::E006, "missing"),
            Diagnostic::new(Code::W101, "unused"),
            Diagnostic::new(Code::W104, "truncated"),
        ];
        let ctx = StatusCtx { diagnostics: &diagnostics, wesl_lines: None };
        let status = StatusInfo::new(true, &ctx, vec!["3 points".into()]);
        assert_eq!(status, StatusInfo { dirty: true, errors: 1, warnings: 2, counts: vec!["3 points".into()] });
        assert_eq!(StatusInfo::new(false, &StatusCtx::default(), Vec::new()), StatusInfo::default());
    }
}
