//! Property widgets of the right sidebar (spec §9.4) and of inline editors on nodes, Material half: the widgets
//! over `ParamKind` / `PinType` ([`value_widget`], [`select_w`], [`pin_kind`]), the slot picker and color swatches.
//! The core widgets editing a [`PropKey`] and their observers are in [`editor_core::ui::props`] (an edit inside a
//! canvas node ends with `CommitInline` through `node_graph_base::InlineEditsPlugin`). The rows each editor kind
//! shows are built in `editors/<kind>.rs` (decision 0031), grouped into subpanes
//! ([`super::widgets::subpane_box`]).

use bevy::feathers::controls::{
    ColorSwatchValue, FeathersColorSwatch, FeathersListView, FeathersSelect, list_rows_from_strings,
};
use bevy::prelude::*;
use bevy::ui::Selected;
use bevy::ui_widgets::ValueChange;
use editor_core::dynval::{PropKey, PropValue};
use shader_graph_core::registry::ParamKind;
use shader_graph_core::{PinType, Value};

use super::gradient_strip::{GradientStrip, slot_list_row};
use super::{TEXT, text};
use crate::ops::prelude::*;
use crate::ops::{Actions, Editor, GraphProp};
pub use editor_core::ui::props::{
    PropField, PropWidgetsPlugin, checkbox_row, checkbox_w, hotkey_row, hue_input, integer, labeled, linear_color,
    number, number_input, number_ranged, select_values, set_action, text_area, text_hint, text_w, vector_fields,
};

/// Color preview kept in sync with the model.
#[derive(Component, Default, Clone)]
pub struct SwatchOf(pub PropKey);

pub struct PropsPlugin;

impl Plugin for PropsPlugin {
    fn build(&self, app: &mut App) {
        // The window adds the core widgets' plugin first (`DomainWindowPlugin`); UI tests add this one alone.
        if !app.is_plugin_added::<PropWidgetsPlugin>() {
            app.add_plugins(PropWidgetsPlugin);
        }
        app.add_systems(Update, update_swatches).add_observer(pick_slot);
    }
}

/// A row editing a value of `kind`: [`labeled`] [`value_widget`], or a [`checkbox_row`] for a bool.
pub(crate) fn value_row(
    label: String,
    target: impl Into<PropKey>,
    kind: &ParamKind,
    v: &Value,
    choices: &Choices,
) -> Box<dyn Scene> {
    let target = target.into();
    match kind {
        ParamKind::Bool => checkbox_row(&label, target, v.as_bool()),
        // The slot list takes the full width, under its label.
        ParamKind::GradientSlot => {
            let widget = value_widget(target, kind, v, choices);
            Box::new(bsn! {
                Node { flex_direction: FlexDirection::Column, row_gap: px(4) }
                Children [ @text(label, TEXT, 13.0) -- @{widget} ]
            })
        }
        _ => Box::new(labeled(label, value_widget(target, kind, v, choices))),
    }
}

fn vector(target: PropKey, v: &[f32], color: bool) -> Box<dyn Scene> {
    let mut items = vector_fields(&target, v, color);
    if color {
        let c = linear_color(v);
        let sw = SwatchOf(target);
        items.insert(0, Box::new(bsn! { @FeathersColorSwatch ColorSwatchValue(c) Node { width: px(22), height: px(22), flex_shrink: 0.0 } sw }));
    }
    Box::new(
        bsn! { Node { flex_direction: FlexDirection::Row, column_gap: px(3), flex_grow: 1.0 } Children [ {items} ] },
    )
}

/// A select of `options`; for a pin-type select `types` are the types behind them (sent as their labels, which the
/// graph turns back into types).
pub(crate) fn select_w(
    target: impl Into<PropKey>,
    options: Vec<String>,
    types: Vec<PinType>,
    current: usize,
) -> Box<dyn Scene> {
    select_values(target, options, types.iter().map(|t| t.label().to_string()).collect(), current)
}

/// Select of `(stored value, label)` choices with a leading `(none)` (= empty string): named route declarations
/// (node ids), curve assets (paths), global constants (names), Texture assets, gradient collections (paths).
fn keyed_select(target: PropKey, choices: &[(String, String)], current: &str) -> Box<dyn Scene> {
    let options: Vec<String> =
        std::iter::once("(none)".to_string()).chain(choices.iter().map(|r| r.1.clone())).collect();
    let values: Vec<String> = std::iter::once(String::new()).chain(choices.iter().map(|r| r.0.clone())).collect();
    let cur = values.iter().position(|v| v == current).unwrap_or(0);
    let rows = list_rows_from_strings(options.clone(), Some(cur));
    let field = PropField { target, options, values, ..Default::default() };
    Box::new(bsn! { @FeathersSelect { @options: rows, @max_visible: 10 } Node { flex_grow: 1.0 } field })
}

/// Choices of reference parameters: named route declarations of the asset, curve assets of the project, global
/// constants (of the parameter's type), Texture assets and built-in textures, gradient collections and the slots of
/// the node's collection.
#[derive(Default, Clone)]
pub(crate) struct Choices {
    pub routes: Vec<(String, String)>,
    pub curves: Vec<(String, String)>,
    pub const_tables: Vec<(String, String)>,
    pub consts: Vec<(String, String)>,
    pub textures: Vec<(String, String)>,
    pub gradients: Vec<(String, String)>,
    pub slots: GradientSlots,
}

/// What a `GradientSlot` parameter can pick (gradient design §6): the slots of the node's collection that hold a
/// gradient (and the picked one even if it is empty), in collection order.
#[derive(Default, Clone)]
pub(crate) struct GradientSlots {
    /// The collection's asset path; empty without a loadable collection.
    pub collection: String,
    pub slots: Vec<SlotEntry>,
}

/// One pickable slot: its position in the collection, id and name.
#[derive(Clone)]
pub(crate) struct SlotEntry {
    pub pos: usize,
    pub id: u32,
    pub name: String,
}

/// The slot list of a `GradientSlot` parameter (a `FeathersListView`) and the parameter it sets.
#[derive(Component, Default, Clone)]
pub struct SlotPicker(pub PropKey);

/// A row of a [`SlotPicker`]: the slot id it picks (`0`: none).
#[derive(Component, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotChoice(pub u32);

/// Ink of the "None" row's name, on its white strip.
const NONE_INK: Color = Color::srgb(0.15, 0.15, 0.15);

/// The slot list of a `GradientSlot` parameter (gradient design §6): "None" (a white strip, as the node outputs
/// without a gradient), then each pickable slot as a [`GradientStrip`] with its name; the row of `current` is
/// `Selected` (the sidebar is refilled after each pick, so the model stays the source of truth).
fn slot_picker(target: PropKey, current: u32, slots: &GradientSlots) -> Box<dyn Scene> {
    let row = |id: u32| -> Box<dyn Scene> {
        let choice = SlotChoice(id);
        if id == current { Box::new(bsn! { choice Selected }) } else { Box::new(bsn! { choice }) }
    };
    let mut rows: Vec<Box<dyn SceneList>> = vec![slot_list_row(
        String::new(),
        row(0),
        Box::new(bsn! { BackgroundColor(Color::WHITE) }),
        Box::new(bsn! { Text("None") TextColor(NONE_INK) TextShadow { color: Color::NONE } }),
    )];
    for s in &slots.slots {
        let strip = GradientStrip { collection: slots.collection.clone(), slot: s.id };
        let name = s.name.clone();
        rows.push(slot_list_row(
            s.pos.to_string(),
            row(s.id),
            Box::new(bsn! { strip ImageNode }),
            Box::new(bsn! { Text(name) }),
        ));
    }
    let rows: Box<dyn SceneList> = Box::new(rows);
    let picker = SlotPicker(target);
    Box::new(bsn! { @FeathersListView { @rows: rows } picker Node { flex_grow: 1.0, max_height: px(220) } })
}

/// A row picked in a [`SlotPicker`] (click, or Space / Enter on the list's active row) sets its parameter.
fn pick_slot(
    ev: On<ValueChange<Entity>>,
    pickers: Query<&SlotPicker>,
    rows: Query<&SlotChoice>,
    mut actions: ResMut<Actions>,
) {
    if let (Ok(picker), Ok(choice)) = (pickers.get(ev.source), rows.get(ev.value)) {
        let value = PropValue::Int(i32::try_from(choice.0).unwrap_or(0));
        actions.push(set_action(&picker.0, value, false));
    }
}

/// Widget editing a value of `kind` (a bool: an uncaptioned checkbox, for rows that show the label themselves).
pub(crate) fn value_widget(target: PropKey, kind: &ParamKind, v: &Value, choices: &Choices) -> Box<dyn Scene> {
    match kind {
        ParamKind::Route => keyed_select(target, &choices.routes, v.as_str()),
        ParamKind::Curve => keyed_select(target, &choices.curves, v.as_str()),
        // `(none)`: the node outputs 1.0 (`false` for flags, W106); picking a table resets the constant.
        ParamKind::GlobalConsts => keyed_select(target, &choices.const_tables, v.as_str()),
        ParamKind::GlobalConst(_) => {
            let cur = u32::try_from(v.as_i32()).ok().filter(|id| *id != 0).map_or(String::new(), |id| id.to_string());
            keyed_select(target, &choices.consts, &cur)
        }
        // `(none)`: the node uses the default texture (W110).
        ParamKind::Texture => keyed_select(target, &choices.textures, v.as_str()),
        // `(none)`: the node outputs white (W111); picking a collection resets the slot (gradient design §6).
        ParamKind::GradientCollection => keyed_select(target, &choices.gradients, v.as_str()),
        ParamKind::GradientSlot => slot_picker(target, u32::try_from(v.as_i32()).unwrap_or(0), &choices.slots),
        ParamKind::Float { min, max } => number_ranged(target, 0, v.as_f32(), soft_range(*min, *max), None),
        ParamKind::Float2 => vector(target, &v.coerce(PinType::Float2).floats(), false),
        ParamKind::Float3 => vector(target, &v.coerce(PinType::Float3).floats(), false),
        ParamKind::Float4 => vector(target, &v.coerce(PinType::Float4).floats(), false),
        ParamKind::Color => vector(target, &v.coerce(PinType::Color).floats(), true),
        ParamKind::Bool => checkbox_w(target, v.as_bool()),
        ParamKind::Int { .. } => integer(target, v.as_i32()),
        ParamKind::Str => text_w(target, v.as_str().to_string(), false),
        ParamKind::Code => text_w(target, v.as_str().to_string(), true),
        ParamKind::Enum(opts) => {
            let cur = opts.iter().position(|o| o == v.as_str()).unwrap_or(0);
            select_w(target, opts.clone(), Vec::new(), cur)
        }
        ParamKind::PinType(types) => {
            let cur = types.iter().position(|t| Some(*t) == v.as_type()).unwrap_or(0);
            select_w(target, types.iter().map(|t| t.label().to_string()).collect(), types.clone(), cur)
        }
    }
}

/// The soft limit of a Float with `min` and `max` (`min < max`).
pub(crate) fn soft_range(min: Option<f32>, max: Option<f32>) -> Option<(f32, f32)> {
    match (min, max) {
        (Some(min), Some(max)) if min < max => Some((min, max)),
        _ => None,
    }
}

/// The widget kind editing an input default of type `t` (`None`: the type has no value editor).
pub(crate) fn pin_kind(t: PinType) -> Option<ParamKind> {
    Some(match t {
        PinType::Float => ParamKind::Float { min: None, max: None },
        PinType::Float2 => ParamKind::Float2,
        PinType::Float3 => ParamKind::Float3,
        PinType::Float4 => ParamKind::Float4,
        PinType::Color => ParamKind::Color,
        PinType::RuntimeBool | PinType::CompileBool => ParamKind::Bool,
        PinType::Int => ParamKind::Int { min: None, max: None },
        PinType::Texture2D | PinType::MaterialAttributesPbr | PinType::Any => return None,
    })
}

fn update_swatches(editor: Res<Editor>, mut q: Query<(&SwatchOf, &mut ColorSwatchValue)>) {
    if !editor.is_changed() {
        return;
    }
    let Some(d) = editor.doc() else { return };
    for (sw, mut v) in &mut q {
        let val = match sw.0.downcast::<GraphProp>() {
            Some(GraphProp::Param { node, key }) => d.asset.nodes.get(node).and_then(|n| n.params.get(key)).cloned(),
            Some(GraphProp::Input { node, pin }) => d.asset.nodes.get(node).and_then(|n| n.inputs.get(pin)).cloned(),
            _ => None,
        };
        if let Some(val) = val {
            let c = linear_color(&val.coerce(PinType::Color).floats());
            if v.0 != c {
                v.0 = c;
            }
        }
    }
}
