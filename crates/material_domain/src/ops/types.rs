use super::*;

pub use editor_core::dynval::display_value;

/// What a property widget of a graph tab edits (the active graph's asset or one of its nodes / comments).
#[derive(Debug, Clone, PartialEq)]
pub enum GraphProp {
    Param {
        node: NodeId,
        key: String,
    },
    Input {
        node: NodeId,
        pin: String,
    },
    Blend,
    TwoSided,
    Shading,
    /// Pin layout of the Pbr3d output node (decision 0023).
    OutputMode,
    Description,
    /// Preview mode of the active material (decision 0022).
    PreviewMode,
    /// Editor-only label of a constant node (decision 0021).
    NodeLabel(NodeId),
    /// Editor-only comment above any node (`Node::comment`, at most `Node::COMMENT_MAX` characters).
    NodeComment(NodeId),
    FrameTitle(String),
    /// Hue of a comment frame (`Frame::hue`, design §4.6): a `Float` in degrees, a `Str` when unset (any non-float
    /// value clears it: "Default colour").
    FrameHue(String),
}

impl PropTarget for GraphProp {
    fn max_chars(&self) -> Option<usize> {
        match self {
            GraphProp::NodeComment(_) => Some(shader_graph_core::model::Node::COMMENT_MAX),
            _ => None,
        }
    }
}

/// What a property widget of a curve tab edits, and the curve tab's edits: `curve_kit`'s (decision 0015), tagged with
/// Material's curve domain (design §14.4).
pub use crate::curve::{CurveAction, CurveEdit, CurveKey, CurveProp};

/// What a property widget of a global constant table edits (decision 0017).
#[derive(Debug, Clone, PartialEq)]
pub enum ConstsProp {
    /// Name of a constant (by its `GlobalConst::id`).
    Name(u32),
    /// Value of a constant (by its `GlobalConst::id`).
    Value(u32),
    /// Type of a constant (by its `GlobalConst::id`; a change may ask first).
    Type(u32),
    /// Name in the add form.
    DraftName,
    /// Type in the add form.
    DraftType,
}

impl PropTarget for ConstsProp {}

/// What a property widget of a Texture or image tab edits: `texture_kit`'s (decision 0024), tagged with Material's
/// texture domain (design §14.4).
pub use crate::texture::{TextureKey, TextureProp};

/// What a property widget of a gradients tab edits: fields of its selected slot (gradient design §8).
#[derive(Debug, Clone, PartialEq)]
pub enum GradientProp {
    /// A field of stop `.0`: component `.1` = 0 is its `t` (a `Float`).
    Stop(usize, u8),
    /// Midpoint of segment `.0` (stop `.0` → `.0 + 1`) (a `Float`).
    Mid(usize),
    /// Name of the slot (a `Str`).
    Name,
    /// Linear RGBA color of stop `.0` (a `Color`).
    Color(usize),
}

impl PropTarget for GradientProp {}

/// A model value as the core property widgets carry it (a pin type as its label).
pub fn prop_value(v: &Value) -> PropValue {
    match v {
        Value::Float(x) => PropValue::Float(*x),
        Value::Float2(x) => PropValue::Float2(*x),
        Value::Float3(x) => PropValue::Float3(*x),
        Value::Float4(x) => PropValue::Float4(*x),
        Value::Color(x) => PropValue::Color(*x),
        Value::Bool(b) => PropValue::Bool(*b),
        Value::Int(i) => PropValue::Int(*i),
        Value::Str(s) => PropValue::Str(s.clone()),
        Value::Type(t) => PropValue::Str(t.label().to_string()),
    }
}

/// A core property value as a model value (a pin type stays its label; the graph converts it where a param holds a
/// type).
pub fn model_value(v: &PropValue) -> Value {
    match v {
        PropValue::Float(x) => Value::Float(*x),
        PropValue::Float2(x) => Value::Float2(*x),
        PropValue::Float3(x) => Value::Float3(*x),
        PropValue::Float4(x) => Value::Float4(*x),
        PropValue::Color(x) => Value::Color(*x),
        PropValue::Bool(b) => Value::Bool(*b),
        PropValue::Int(i) => Value::Int(*i),
        PropValue::Str(s) => Value::Str(s.clone()),
    }
}

/// [`TabAction::SetValue`] of a Material property with a model value.
pub fn set_value(target: impl PropTarget, value: Value, merge: bool) -> Action {
    Action::Tab(TabAction::SetValue { key: PropKey::new(target), value: prop_value(&value), merge })
}

/// [`TabAction::SetComponent`] of a Material property.
pub fn set_component(target: impl PropTarget, comp: usize, value: f32, is_final: bool) -> Action {
    Action::Tab(TabAction::SetComponent { key: PropKey::new(target), comp, value, is_final })
}

/// Edits of the active gradients tab (gradient design §8), one per [`crate::gradients_doc::GradientsDoc`] method.
/// Edits with a key (`Some("drag".into())`) merge with the previous one of the same key into one undo step until
/// [`TabAction::CommitInline`].
#[derive(Debug, Clone, PartialEq)]
pub enum GradientsAction {
    /// Selects the slot at this position.
    SelectSlot(usize),
    /// Selects a stop or a midpoint of the selected slot.
    Select(Option<GradientSel>),
    /// Replaces the selected slot's stops and the selection.
    SetStops(Vec<Stop>, Option<GradientSel>, Option<MergeKey>),
    /// Adds a stop at this `t` (refused with a log message past `MAX_STOPS`).
    AddStop(f32),
    /// Deletes the selected stop.
    DeleteSelected,
    /// Midpoint of a segment.
    SetMid(usize, f32, Option<MergeKey>),
    /// `t` of a stop (it stays selected through the re-sort).
    SetStopT(usize, f32, Option<MergeKey>),
    /// Linear RGBA color of a stop.
    SetStopColor(usize, [f32; 4], Option<MergeKey>),
    /// Name of the selected slot.
    SetName(String, Option<MergeKey>),
    /// Moves a slot from one position to another.
    MoveSlot(usize, usize),
    /// Empties the selected slot (its id stays).
    ClearSlot,
    /// Copies the selected slot into the next empty one (refused with a log message when none is).
    DuplicateSlot,
    /// Copies the selected slot (stops + name) to the gradient clipboard.
    CopySlot,
    /// Pastes the gradient clipboard into the selected slot (it keeps its id).
    PasteSlot,
}

/// Edits of the active global constant table (decision 0017).
#[derive(Debug, Clone, PartialEq)]
pub enum ConstsAction {
    /// Adds the constant of the add form.
    Add,
    /// Changes a constant's type; `confirmed` once the user agreed to clear the references that break.
    SetType { id: u32, ty: ConstType, confirmed: bool },
    /// Deletes a constant; `confirmed` once the user agreed to clear its references.
    Delete { id: u32, confirmed: bool },
    /// Undo (`undo`) or redo in the table; asks first when the step retypes or removes a constant nodes read.
    History { undo: bool, confirmed: bool },
}

/// Edits of the active Texture tab and the image preview's channel: `texture_kit`'s (decision 0024), tagged with
/// Material's texture domain (design §14.4).
pub use crate::texture::{PreviewChannel, TextureAction, TextureEdit};

/// Kind-specific edits reach their tab as [`TabAction::Doc`] (design §6.2).
macro_rules! doc_edit {
    ($($t:ty),*) => {$(
        impl From<$t> for TabAction {
            fn from(a: $t) -> TabAction {
                TabAction::doc(a)
            }
        }
        impl From<$t> for Action {
            fn from(a: $t) -> Action {
                Action::Tab(TabAction::doc(a))
            }
        }
    )*};
}
doc_edit!(GradientsAction, ConstsAction);

/// A wire end the palette should connect the new node to.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub node: NodeId,
    pub pin: String,
    /// The existing pin is an output (the new node gets an input connected).
    pub from_output: bool,
}

/// The Material domain's commands (design §6.2): `Action::Domain(MATERIAL, …)`, handed to `MaterialState`'s
/// `apply` hook. `From<MaterialAction> for Action` wraps one.
#[derive(Debug, Clone, PartialEq)]
pub enum MaterialAction {
    AddNode {
        kind: String,
        pos: (f32, f32),
        link: Option<Link>,
        /// Preset parameters (a palette entry of one named route).
        params: Vec<(String, Value)>,
    },
    /// [`MaterialAction::AddNode`], then the new node's rename dialog ([`Editor::rename_target`], as a header double click):
    /// a Named Route declaration picked in the palette (design §6.4).
    AddNodeAndRename { kind: String, pos: (f32, f32), link: Option<Link>, params: Vec<(String, Value)> },
    /// Type of a node's polymorphic inputs, stored as the type of their inline values (RMB pin menu).
    SetPinType { node: NodeId, ty: PinType },
    /// Drops an inline value override; the pin falls back to its default.
    ResetInput { node: NodeId, pin: String },
    /// Changes the kind of nodes in place (node menu / hotkey "Convert to", decision 0021).
    ConvertNodes { nodes: Vec<NodeId>, conv: Conversion },
    /// Something dragged from a sidebar tree dropped onto the active graph at `pos` (graph space), over `node` if
    /// any (design §4.3): a node with a matching param gets the reference, otherwise a new node is added at `pos`.
    Drop { item: super::droppable::Dropped, pos: (f32, f32), node: Option<NodeId> },
    /// Previews a node of the active material, or stops previewing it (node menu / `V`, decision 0022).
    TogglePreview(NodeId),
    /// Asks for the text of a node's name / label or a comment's title (double click, design §5.5); the dialog
    /// submits it as a [`TabAction::SetValue`] of `target`.
    TextDialog(GraphProp),
    /// Opens an asset's file in the user's default application: the file generated in language `Some(`[`WESL`] /
    /// [`RUST`]`)`, or the asset's own `.ron` for `None`.
    ShowFile(AssetId, Option<&'static str>),
    /// Opens the file generated from an asset in language [`WESL`] / [`RUST`] in a read-only text tab.
    ShowGenerated(AssetId, &'static str),
    /// Selects `node` in the active graph and centers the canvas on it (double-clicking the header of a Named Route
    /// usage node jumps to its declaration).
    FocusNode(NodeId),
    /// Opens a gradient collection asset in a tab and selects the slot with stable `slot_id` (double-clicking a
    /// `Sample Gradient` node or clicking its "Open gradients" button).
    OpenGradient { id: AssetId, slot_id: u32 },
}

impl From<MaterialAction> for Action {
    fn from(a: MaterialAction) -> Action {
        Action::Domain(MATERIAL, Payload::new(a))
    }
}

/// The Material command an action wraps.
pub fn material_action(a: &Action) -> Option<&MaterialAction> {
    match a {
        Action::Domain(id, p) if *id == MATERIAL => p.downcast(),
        _ => None,
    }
}

pub use editor_core::action::{Action, Actions, CloseTabs, Dialog, Submit, TreeId, Unsaved, ViewRequest};
pub use editor_core::message::{Level, Message};
pub use node_graph_base::GraphEdit;

pub use editor_core::state::editor::{Editor, UiRev};
