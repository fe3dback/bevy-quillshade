//! The seam between the graph UI and a domain's graph model (design §3.3, §7.3): [`GraphDomain`]. The base owns the
//! canvas, its gestures, the palette shell, the in-node preview mechanism, the minimap, the graph editor topbar tools,
//! the base input context and the frame / multi-selection subpanes; everything that knows the model is a call here.

use bevy::prelude::*;
use editor_core::action::Action;
use editor_core::doc_kind::TabDoc;
use editor_core::dynval::PropKey;
use editor_core::menu::MenuEntry;
use editor_core::state::editor::Editor;
use editor_core::tab::ViewKindId;

use crate::edit::PinRef;
use crate::view::{
    DiagMark, DropItem, FrameField, GraphHotkey, GraphView, MovePlan, NodeLook, PaletteEntry, PlaceSpec, PreviewRow,
};

/// What the [`GraphDomain`] calls may read besides the document.
pub struct GraphCtxRef<'a> {
    pub editor: &'a Editor,
}

/// A domain's node graph (design §7.3). Registered with [`crate::register_graph`]; the canvas shows the active tab
/// while its document is a [`GraphDomain::Doc`]. Node and pin ids are opaque strings to the base; edits come back as
/// [`crate::GraphEdit`]s through [`TabDoc::apply`].
pub trait GraphDomain: Send + Sync + 'static {
    /// The domain's graph document.
    type Doc: TabDoc;
    /// The editor kind of [`GraphDomain::Doc`] ([`TabDoc::view`]).
    const VIEW: ViewKindId;

    /// The graph to draw.
    fn view(doc: &Self::Doc) -> GraphView<'_>;
    /// How node `node` (one of [`GraphDomain::view`]'s) looks.
    fn node_look(ctx: &GraphCtxRef, doc: &Self::Doc, node: &str) -> NodeLook;
    /// The diagnostics shown on the canvas now (outlines, pin labels); read every frame.
    fn diagnostics(ctx: &GraphCtxRef, doc: &Self::Doc) -> Vec<DiagMark>;
    /// Whether a wire `from` (an output) → `to` (an input) may connect; `Err` says why not.
    fn can_connect(ctx: &GraphCtxRef, doc: &Self::Doc, from: &PinRef, to: &PinRef) -> Result<(), String>;
    /// Whether the wires of `from` may move to `to` (a Ctrl+drag, decision 0032); changes nothing.
    fn plan_move_wires(ctx: &GraphCtxRef, doc: &Self::Doc, from: &PinRef, to: &PinRef) -> MovePlan;
    /// A double click on the node's header (rename dialog, open the referenced asset, …; design §5.5).
    fn header_action(ctx: &GraphCtxRef, doc: &Self::Doc, node: &str) -> Option<Action>;
    /// A double click on the node's body (open the referenced asset).
    fn open_action(ctx: &GraphCtxRef, doc: &Self::Doc, node: &str) -> Option<Action>;
    /// The menu of an input pin (right click); empty: none opens.
    fn pin_menu(ctx: &GraphCtxRef, doc: &Self::Doc, pin: &PinRef) -> Vec<MenuEntry>;
    /// The menu of the clicked node `nodes[0]` (right click; it is selected first).
    fn node_menu(ctx: &GraphCtxRef, doc: &Self::Doc, nodes: &[String]) -> Vec<MenuEntry>;
    /// The actions of a node action hotkey on the selection (decision 0021).
    fn hotkey_actions(ctx: &GraphCtxRef, doc: &Self::Doc, key: GraphHotkey) -> Vec<Action>;
    /// Hold a key and click the empty canvas to drop a node (decision 0032): the keys and what they drop.
    fn placement_hotkeys() -> &'static [(KeyCode, PlaceSpec)];
    /// The action adding a [`PlaceSpec`] node at `at` (graph space); `None`: nothing is placed.
    fn place(ctx: &GraphCtxRef, doc: &Self::Doc, spec: PlaceSpec, at: Vec2) -> Option<Action>;
    /// The palette's entries, by category and name; `link`: the wire end it was opened from (entries compatible
    /// with it only).
    fn palette(ctx: &GraphCtxRef, doc: &Self::Doc, link: Option<&PinRef>) -> Vec<PaletteEntry>;
    /// A tree row dropped onto the canvas at `pos` (graph space), over node `over` if any.
    fn drop(ctx: &GraphCtxRef, doc: &Self::Doc, item: &DropItem, pos: Vec2, over: Option<&str>) -> Option<Action>;
    /// The right sidebar for one selected node or none; the base shows the frame and multi-selection subpanes itself.
    fn sidebar(ctx: &GraphCtxRef, doc: &Self::Doc) -> Vec<Box<dyn Scene>>;
    /// The property a field of the comment frame `frame` edits (its Title / Hue in the right sidebar).
    fn frame_prop(frame: &str, field: FrameField) -> PropKey;
    /// Scenes after the base tools in the editor topbar, spawned once with it (a domain fills its own parts).
    fn topbar_extras() -> Vec<Box<dyn Scene>>;
    /// The in-node preview rows of `node`.
    fn previews(ctx: &GraphCtxRef, doc: &Self::Doc, node: &str) -> Vec<PreviewRow>;
    /// Bindings the domain adds to the graph's input context (after the base ones).
    fn bindings() -> Vec<Box<dyn Scene>> {
        Vec::new()
    }
    /// The domain's own systems of the graph editor's areas (`EditorAreas::build`).
    fn build(_app: &mut App) {}
    /// The domain's handlers of its [`GraphDomain::bindings`] (`EditorAreas::input`).
    fn input(_app: &mut App) {}
}

/// The active tab's document, when it is `G`'s graph.
pub fn active<G: GraphDomain>(editor: &Editor) -> Option<&G::Doc> {
    editor.active_doc::<G::Doc>()
}
