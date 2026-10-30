//! What the graph canvas asks of the domain's document (design §7.3): [`GraphEdit`]s, sent as
//! `Action::Tab(TabAction::Doc(…))` to the active tab, whose document applies them to its model.

use std::collections::BTreeSet;

use bevy::prelude::*;
use editor_core::action::{Action, TabAction, ViewRequest};

/// A model pin: the port entity of a pin on the canvas carries it, and the [`crate::GraphDomain`] calls name pins by
/// it. Node and pin ids are opaque strings to the base (design §7.3).
#[derive(Component, Default, Clone, Debug, PartialEq)]
pub struct PinKey {
    pub node: String,
    pub pin: String,
    pub output: bool,
}

/// A pin as the [`crate::GraphDomain`] calls name it (the [`PinKey`] of its port).
pub type PinRef = PinKey;

/// An edit the graph canvas and its tools make (design §7.3). The domain's document applies it to its model; the
/// canvas is rebuilt from the model afterwards (one-way sync, spec §14).
#[derive(Debug, Clone, PartialEq)]
pub enum GraphEdit {
    /// A wire from an output `(node, pin)` to an input.
    Connect { from: (String, String), to: (String, String) },
    /// Removes the wire into an input.
    Disconnect { to: (String, String) },
    /// Removes every wire at a pin (Alt+LMB on it).
    DisconnectPin { node: String, pin: String, output: bool },
    /// Moves the wires of a pin to another pin of the same direction (Ctrl+LMB drag, decision 0032): an input's
    /// single wire to the input `to`, or every wire of an output to the output `to`. All or nothing, one undo step.
    MoveWires { node: String, pin: String, output: bool, to: (String, String) },
    /// Final node / comment frame positions after a drag (one undo step).
    Moved { nodes: Vec<(String, (f32, f32))>, frames: Vec<(String, (f32, f32))> },
    /// The node selection (clears the selected comment frame).
    Select(BTreeSet<String>),
    /// The selected comment frame.
    SelectFrame(Option<String>),
    /// Frames selected on the canvas together with the node selection (pushed after [`GraphEdit::Select`]); only
    /// those whose nodes are all selected are kept ([`crate::layout::enclosed`], design §4.5).
    SelectFrames(BTreeSet<String>),
    /// A reroute point on the wire `from` → `to` at `pos` (graph space; double click on a wire).
    AddReroute { from: (String, String), to: (String, String), pos: (f32, f32) },
    /// A comment frame around the selection, or with none a default-size one at this graph position snapped to the
    /// grid (design §6.1). `None` (the editor topbar's Comment button, hold C with the cursor off the canvas): the
    /// visible canvas centre, which the graph editor fills in before the action is applied; the origin without a
    /// canvas.
    AddComment(Option<(f32, f32)>),
    /// Asks for the title of a comment frame (double click on its title strip, design §5.5).
    FrameTitle(String),
    /// Arranges the selected nodes (all when none is selected).
    AutoLayout,
    /// A camera request the canvas carries out once layout is known.
    View(ViewRequest),
}

impl From<GraphEdit> for TabAction {
    fn from(e: GraphEdit) -> TabAction {
        TabAction::doc(e)
    }
}

impl From<GraphEdit> for Action {
    fn from(e: GraphEdit) -> Action {
        Action::Tab(TabAction::doc(e))
    }
}

/// The graph edit an action carries, if any.
pub fn graph_edit(a: &Action) -> Option<&GraphEdit> {
    match a {
        Action::Tab(TabAction::Doc(p)) => p.downcast(),
        _ => None,
    }
}
