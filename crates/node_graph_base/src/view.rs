//! What a [`crate::GraphDomain`] shows on the canvas (design §7.3): the graph's data ([`GraphView`]), the look of
//! each node ([`NodeLook`]) and the live diagnostics marks ([`DiagMark`]). Pure data: the canvas
//! ([`crate::canvas`]) builds the bevy_noodle scene from it.

use std::collections::BTreeSet;
use std::sync::Arc;

use bevy::prelude::*;
use editor_core::action::Action;
use editor_core::search::Searchable;

use crate::edit::PinRef;

/// The graph the canvas shows: nodes, wires, comment frames and the selection.
pub struct GraphView<'a> {
    /// In the order they are spawned (later ones are drawn on top).
    pub nodes: Vec<NodeView>,
    /// Wires as (output pin, input pin).
    pub edges: Vec<(PinRef, PinRef)>,
    pub frames: Vec<FrameView>,
    pub selection: &'a BTreeSet<String>,
    pub selected_frame: Option<&'a str>,
    /// Comment frames selected together with the node selection (design §4.5).
    pub enclosed_frames: &'a BTreeSet<String>,
}

/// A node of a [`GraphView`]: its id and its top-left corner (graph space).
#[derive(Clone, Debug, PartialEq)]
pub struct NodeView {
    pub id: String,
    pub pos: Vec2,
}

/// A comment frame of a [`GraphView`].
#[derive(Clone, Debug, PartialEq)]
pub struct FrameView {
    pub id: String,
    /// `(x, y, width, height)`, graph space.
    pub rect: (f32, f32, f32, f32),
    pub title: String,
    /// Hue in degrees (design §4.6); `None`: [`FrameView::color`] or the default colour.
    pub hue: Option<f32>,
    /// A stored sRGBA colour (older files), used while there is no hue.
    pub color: Option<[f32; 4]>,
    /// The nodes the frame was made around: a box selecting all of them may delete it with them (design §4.5).
    pub nodes: Vec<String>,
}

/// The shape of a node on the canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NodeShape {
    /// A titled box with one row per pin.
    #[default]
    Node,
    /// A pill holding its first input and first output pin (a reroute point).
    Pill,
}

/// How a node is drawn.
#[derive(Default)]
pub struct NodeLook {
    pub title: String,
    /// The title bar's colour.
    pub header: Color,
    pub shape: NodeShape,
    /// Outputs and inputs, each kind in the order of its rows (outputs above inputs).
    pub pins: Vec<PinLook>,
    /// Inline value editors of input pins: `(pin id, editor)`, shown in the pin's row.
    pub inline: Vec<(String, Box<dyn Scene>)>,
    /// A row of the node's own value editor (a constant's value), between the previews and the pins.
    pub body: Option<Box<dyn Scene>>,
    /// The node holds wide editors (one number field per vector component): a wider minimum width.
    pub wide: bool,
    /// Shown in the error colour instead of the pins (an unknown node kind).
    pub note: Option<String>,
    pub emphasis: Emphasis,
    /// The node's comment, drawn in a box above it.
    pub comment: Option<String>,
}

/// Emphasis of a node fixed when the canvas is built. Diagnostics change without a new revision (validation runs
/// when the editor is idle), so they are not part of it: [`crate::GraphDomain::diagnostics`] is read every frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Emphasis {
    /// The previewed node (decision 0022): a pulsing outline and a warning icon in the title.
    pub pulse: bool,
}

/// One pin of a [`NodeLook`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PinLook {
    pub id: String,
    /// The text of its row.
    pub label: String,
    /// The colour of its dot (its type).
    pub color: Color,
    pub output: bool,
    /// Fed by the context when unconnected (no inline editor).
    pub implicit: bool,
    /// Must be wired (no inline editor).
    pub required: bool,
    /// Ignored by the node's current settings: a dimmed label.
    pub inactive: bool,
    /// Not drawn (a pin fed by something other than a wire).
    pub hidden: bool,
}

/// A diagnostic shown on the canvas: the node gets a warning / error outline, the pin (if any) a coloured label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagMark {
    pub node: String,
    /// `None`: the node only (its pin labels keep their colour).
    pub pin: Option<String>,
    pub error: bool,
}

/// One in-node preview row of a node ([`crate::preview`]).
pub struct PreviewRow {
    pub scene: Box<dyn Scene>,
    /// A column left of the node's output pins instead of a full-width row above them (when the node has outputs).
    pub beside_outputs: bool,
}

/// What picking a palette entry does: the action for a new node at this graph position.
pub type PickFn = Arc<dyn Fn(Vec2) -> Action + Send + Sync>;

/// A row of the node palette ([`crate::palette`]).
#[derive(Clone)]
pub struct PaletteEntry {
    /// What the entry adds, in the domain's terms (a node kind); opaque to the base.
    pub id: String,
    pub name: String,
    pub category: String,
    pub keywords: Vec<String>,
    pub description: String,
    /// Wins ties on the search score (the Named Route declaration after a wire from an output, design §6.4).
    pub lead: bool,
    pub pick: PickFn,
}

impl Searchable for PaletteEntry {
    fn name(&self) -> &str {
        &self.name
    }
    fn category(&self) -> &str {
        &self.category
    }
    fn keywords(&self) -> &[String] {
        &self.keywords
    }
    fn description(&self) -> &str {
        &self.description
    }
}

impl std::fmt::Debug for PaletteEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaletteEntry")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("category", &self.category)
            .finish()
    }
}

/// A field of a comment frame in the right sidebar's Comment subpane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameField {
    Title,
    Hue,
}

/// The node action hotkeys (decision 0021): Shift + the main-row key, or the numpad key alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphHotkey {
    Plus,
    Minus,
    Star,
    Slash,
}

/// What a held placement key drops on the canvas (design §7.3): the domain's name of a node kind.
pub type PlaceSpec = &'static str;

/// Whether the wires a Ctrl+drag lifted may move to a pin; `Err` says why not.
pub type MovePlan = Result<(), String>;

/// A row dragged from a sidebar tree onto the canvas.
pub type DropItem = editor_core::ui::tree::TreeRow;
