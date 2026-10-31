//! Core property widgets (design §6.2): number / vector / checkbox / select / text / color fields editing an opaque
//! [`PropKey`], and their observers. A field's change becomes an [`Action::Tab`] of [`TabAction::SetValue`] /
//! [`TabAction::SetComponent`], which the active tab's document downcasts. A field's last edit (a released drag, a
//! toggle, a closed color picker) also triggers [`FieldEditEnded`]: where the field lives may end the edit's undo step
//! there (a graph domain: an inline editor on a canvas node ends with `CommitInline`); core knows no such place.

use bevy::feathers::controls::{
    FeathersCheckbox, FeathersMenuButton, FeathersNumberInput, FeathersSelect, FeathersTextInput,
    FeathersTextInputContainer, HardLimit, NumberInputPrecision, NumberInputRange, NumberInputValue, OptionIndex,
    SoftLimit, list_rows_from_strings,
};
use bevy::feathers::display::caption;
use bevy::feathers::tokens;
use bevy::prelude::*;
use bevy::text::{EditableText, TextEdit, TextEditChange};
use bevy::ui::Checked;
use bevy::ui_widgets::ValueChange;

use crate::action::{Action, Actions, TabAction};
use crate::dynval::{PropKey, PropValue, display_value};
use crate::theme::{TEXT, TEXT_DIM, text};

/// A property widget: what it edits and how to read its value.
#[derive(Component, Default, Clone)]
pub struct PropField {
    pub target: PropKey,
    pub comp: usize,
    /// Option labels of a select.
    pub options: Vec<String>,
    /// For selects whose stored value differs from the label (route node ids, pin types).
    pub values: Vec<String>,
}

/// Last text of a text field that reached the model. `TextEditChange` also fires for cursor moves and while
/// the field is being filled with its initial value; those must not become edits (each would bump the revision).
#[derive(Component, Default, Clone)]
pub struct FieldText {
    last: String,
    /// The initial value has been shown.
    ready: bool,
}

/// Dim hint over an empty text field ([`text_hint`]): Bevy 0.20's text input has no placeholder of its own.
#[derive(Component, Default, Clone)]
pub struct Placeholder;

/// Text to put into a freshly spawned text field.
#[derive(Component, Default, Clone)]
pub struct InitText(pub String);

/// The property widgets' systems and observers: initial texts, placeholders, and the edits of selects, text,
/// number, color and checkbox fields.
pub struct PropWidgetsPlugin;

impl Plugin for PropWidgetsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (init_text, show_placeholders, style_select_widgets))
            .add_observer(select)
            .add_observer(text_changed)
            .add_observer(number_f32)
            .add_observer(number_i32)
            .add_observer(color)
            .add_observer(checkbox_changed);
    }
}

/// The last edit of a number, color or checkbox field `source` was sent (after its `SetComponent` / `SetValue`): a
/// domain whose fields sit somewhere special observes it (a graph domain ends an inline editor's undo step with
/// [`TabAction::CommitInline`]). Triggered through `Commands`, so its observers push their actions right after the
/// field's.
#[derive(Event, Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldEditEnded {
    pub source: Entity,
}

fn init_text(mut q: Query<(&InitText, &mut EditableText), Added<InitText>>) {
    for (init, mut t) in &mut q {
        t.queue_edit(TextEdit::SelectAll);
        t.queue_edit(TextEdit::Insert(init.0.as_str().into()));
    }
}

/// Ensures newly spawned [`FeathersSelect`] widgets keep single-line captions/rows (`LineBreak::NoWrap`) and a gap
/// before the chevron inside [`FeathersMenuButton`].
fn style_select_widgets(
    q_select: Query<Entity, Added<FeathersSelect>>,
    q_children: Query<&Children>,
    q_text: Query<(), With<Text>>,
    mut q_btn: Query<&mut Node, With<FeathersMenuButton>>,
    mut commands: Commands,
) {
    for select_ent in &q_select {
        for desc in q_children.iter_descendants(select_ent) {
            if q_text.contains(desc) {
                commands.entity(desc).insert(TextLayout::linebreak(LineBreak::NoWrap));
            }
            if let Ok(mut node) = q_btn.get_mut(desc) {
                node.column_gap = px(6);
                node.overflow = Overflow::clip();
            }
        }
    }
}

pub fn labeled(label: String, widget: Box<dyn Scene>) -> impl Scene {
    bsn! {
        Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, column_gap: px(6), min_height: px(26) }
        Children [
            Node { width: px(110), flex_shrink: 0.0 } Children [ @text(label, TEXT, 13.0) ]
            --
            Node { flex_grow: 1.0, flex_direction: FlexDirection::Row, column_gap: px(3), align_items: AlignItems::Center } Children [ @{widget} ]
        ]
    }
}

/// One hotkey row: the keys as a key cap, then what they do.
pub fn hotkey_row(keys: &'static str, what: &'static str) -> impl Scene {
    bsn! {
        Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, column_gap: px(8), min_height: px(22) }
        Children [
            Node { width: px(100), flex_shrink: 0.0 }
            Children [
                Node { padding: UiRect::axes(px(5), px(1)), border: UiRect::all(px(1)), border_radius: BorderRadius::all(px(3)) }
                BackgroundColor(Color::srgb_u8(44, 46, 53))
                BorderColor::all(Color::srgb_u8(66, 68, 77))
                Children [ @text(keys, TEXT, 12.0) ]
            ]
            --
            Node { flex_grow: 1.0 } Children [ @text(what, TEXT_DIM, 12.0) ]
        ]
    }
}

/// A `FeathersCheckbox` with `label` as its caption (a click on the label toggles it too) editing the bool `target`:
/// its `ValueChange<bool>` reaches the model through the property observers.
pub fn checkbox(label: &str, target: impl Into<PropKey>, checked: bool) -> Box<dyn Scene> {
    let (label, field) = (label.to_string(), PropField { target: target.into(), ..Default::default() });
    if checked {
        Box::new(
            bsn! { @FeathersCheckbox { @caption: bsn! { @caption(label) } } Node { flex_shrink: 0.0 } Checked field },
        )
    } else {
        Box::new(bsn! { @FeathersCheckbox { @caption: bsn! { @caption(label) } } Node { flex_shrink: 0.0 } field })
    }
}

/// A bool as a captioned checkbox ([`checkbox`]) filling a row as tall as a [`labeled`] one: the box starts where the
/// other rows' labels do and its caption is the clickable label (no empty label column in front of it).
pub fn checkbox_row(label: &str, target: impl Into<PropKey>, checked: bool) -> Box<dyn Scene> {
    let checkbox = checkbox(label, target, checked);
    Box::new(bsn! {
        Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, min_height: px(26) }
        Children [ @{checkbox} ]
    })
}

/// A number input editing component `comp` of `target`; a component's field (`sigil` X/Y/Z/W or R/G/B/A) is
/// marked with its axis colour.
pub fn number(target: impl Into<PropKey>, comp: usize, v: f32, sigil: Option<&'static str>) -> Box<dyn Scene> {
    let Some(s) = sigil else { return number_ranged(target, comp, v, None, None) };
    let field = PropField { target: target.into(), comp, ..Default::default() };
    let v = display_value(v);
    let token = match comp {
        0 => tokens::TEXT_INPUT_X_AXIS,
        1 => tokens::TEXT_INPUT_Y_AXIS,
        2 => tokens::TEXT_INPUT_Z_AXIS,
        _ => tokens::TEXT_INPUT_W_AXIS,
    };
    Box::new(bsn! {
        @FeathersNumberInput { @sigil_color: token, @label_text: s }
        NumberInputValue::F32(v)
        NumberInputPrecision(3)
        Node { flex_grow: 1.0, min_width: px(40) }
        field
    })
}

/// A number input with optional limits (design §3): dragging stays inside the `soft` range (its fill bar shows
/// where the value is), a click enters text editing and a typed value may leave it; a `hard` limit clamps both.
/// Drags round to 2 decimals with a soft limit (as the sliders it replaces did), else to 3 — without a soft limit
/// Feathers also derives the drag speed from the precision (`10^-precision` per pixel).
pub fn number_ranged(
    target: impl Into<PropKey>,
    comp: usize,
    v: f32,
    soft: Option<(f32, f32)>,
    hard: Option<(f32, f32)>,
) -> Box<dyn Scene> {
    let precision = if soft.is_some() { 2 } else { 3 };
    number_input(target, comp, v, soft, hard, precision)
}

/// [`number_ranged`] rounding drags to `precision` decimals.
pub fn number_input(
    target: impl Into<PropKey>,
    comp: usize,
    v: f32,
    soft: Option<(f32, f32)>,
    hard: Option<(f32, f32)>,
    precision: i32,
) -> Box<dyn Scene> {
    let field = PropField { target: target.into(), comp, ..Default::default() };
    let v = display_value(v);
    let soft: Box<dyn Scene> = match soft {
        Some((min, max)) => Box::new(bsn! { SoftLimit(NumberInputRange::F32({min..=max})) }),
        None => Box::new(bsn! {}),
    };
    let hard: Box<dyn Scene> = match hard {
        Some((min, max)) => Box::new(bsn! { HardLimit(NumberInputRange::F32({min..=max})) }),
        None => Box::new(bsn! {}),
    };
    Box::new(bsn! {
        @FeathersNumberInput
        NumberInputValue::F32(v)
        NumberInputPrecision(precision)
        @{soft}
        @{hard}
        Node { flex_grow: 1.0 }
        field
    })
}

pub fn integer(target: impl Into<PropKey>, v: i32) -> Box<dyn Scene> {
    let field = PropField { target: target.into(), ..Default::default() };
    Box::new(bsn! { @FeathersNumberInput NumberInputValue::I32(v) Node { flex_grow: 1.0 } field })
}

/// One number field per component, labelled X/Y/Z/W or R/G/B/A.
pub fn vector_fields(target: &PropKey, v: &[f32], color: bool) -> Vec<Box<dyn Scene>> {
    const XYZW: [&str; 4] = ["X", "Y", "Z", "W"];
    const RGBA: [&str; 4] = ["R", "G", "B", "A"];
    let names = if color { RGBA } else { XYZW };
    v.iter().enumerate().map(|(i, x)| number(target.clone(), i, *x, Some(names[i]))).collect()
}

/// Stored color components are linear (spec §7.1: `Color` → `LinearRgba`).
pub fn linear_color(v: &[f32]) -> Color {
    Color::linear_rgba(v[0], v[1], v[2], v.get(3).copied().unwrap_or(1.0))
}

/// Hue in whole degrees, `0..=360` both soft and hard; a drag is one undo step (`SetComponent`, `is_final` at the
/// end).
pub fn hue_input(target: impl Into<PropKey>, hue: f32) -> Box<dyn Scene> {
    number_input(target, 0, hue, Some((0.0, 360.0)), Some((0.0, 360.0)), 0)
}

/// An uncaptioned checkbox, where the label is elsewhere (an inline editor on a node, a consts table cell); a
/// labelled row uses [`checkbox_row`].
pub fn checkbox_w(target: impl Into<PropKey>, v: bool) -> Box<dyn Scene> {
    let field = PropField { target: target.into(), ..Default::default() };
    if v {
        Box::new(bsn! { @FeathersCheckbox { @caption: bsn! { @caption("") } } Checked field })
    } else {
        Box::new(bsn! { @FeathersCheckbox { @caption: bsn! { @caption("") } } field })
    }
}

/// A select of `options`, sending the picked label, or the entry of `values` at its position when given.
pub fn select_values(
    target: impl Into<PropKey>,
    options: Vec<String>,
    values: Vec<String>,
    current: usize,
) -> Box<dyn Scene> {
    let rows = list_rows_from_strings(options.clone(), Some(current));
    let field = PropField { target: target.into(), options, values, ..Default::default() };
    Box::new(bsn! { @FeathersSelect { @options: rows, @max_visible: 10 } Node { flex_grow: 1.0 } field })
}

pub fn text_w(target: impl Into<PropKey>, v: String, multiline: bool) -> Box<dyn Scene> {
    let sync = FieldText { ready: v.is_empty(), last: v.clone() };
    let (field, init) = (PropField { target: target.into(), ..Default::default() }, InitText(v));
    if multiline {
        Box::new(bsn! {
            @FeathersTextInputContainer
            Node { height: auto(), min_height: px(160), flex_grow: 0.0, width: percent(100), align_items: AlignItems::FlexStart, padding: UiRect::all(px(4)) }
            Children [
                @FeathersTextInput
                EditableText { cursor_width: 0.3, allow_newlines: true, visible_lines: {Some(12.0)} }
                TextLayout { linebreak: LineBreak::WordOrCharacter }
                field
                init
                sync
            ]
        })
    } else {
        Box::new(
            bsn! { @FeathersTextInputContainer Node { flex_grow: 0.0, width: percent(100) } Children [ @FeathersTextInput field init sync ] },
        )
    }
}

/// A short multiline [`text_w`] (a few lines, word-wrapped, Enter breaks the line) taking at most `max` characters.
pub fn text_area(target: impl Into<PropKey>, v: String, max: usize) -> Box<dyn Scene> {
    let sync = FieldText { ready: v.is_empty(), last: v.clone() };
    let (field, init) = (PropField { target: target.into(), ..Default::default() }, InitText(v));
    Box::new(bsn! {
        @FeathersTextInputContainer
        Node { height: auto(), min_height: px(64), flex_grow: 0.0, width: percent(100), align_items: AlignItems::FlexStart, padding: UiRect::all(px(4)) }
        Children [
            @FeathersTextInput
            EditableText { cursor_width: 0.3, allow_newlines: true, visible_lines: {Some(6.0)}, max_characters: {Some(max)} }
            TextLayout { linebreak: LineBreak::WordOrCharacter }
            field
            init
            sync
        ]
    })
}

/// A one-line [`text_w`] showing `hint` while it is empty.
pub fn text_hint(target: impl Into<PropKey>, v: String, hint: &str) -> Box<dyn Scene> {
    let sync = FieldText { ready: v.is_empty(), last: v.clone() };
    let vis = if v.is_empty() { Visibility::Inherited } else { Visibility::Hidden };
    let (field, init, hint) =
        (PropField { target: target.into(), ..Default::default() }, InitText(v), hint.to_string());
    Box::new(bsn! {
        @FeathersTextInputContainer
        Node { flex_grow: 0.0, width: percent(100) }
        Children [
            @FeathersTextInput field init sync
            --
            Placeholder
            Text(hint)
            TextFont { font_size: FontSize::Px(13.0) }
            TextColor(TEXT_DIM)
            Node { position_type: PositionType::Absolute, left: px(8) }
            Pickable::IGNORE
            vis
        ]
    })
}

/// Shows each [`Placeholder`] exactly while the text field next to it is empty.
fn show_placeholders(
    mut hints: Query<(&ChildOf, &mut Visibility), With<Placeholder>>,
    children: Query<&Children>,
    texts: Query<&EditableText>,
) {
    for (parent, mut vis) in &mut hints {
        let siblings = children.get(parent.parent()).into_iter().flatten();
        let empty = siblings.filter_map(|c| texts.get(*c).ok()).all(|t| t.value().to_string().is_empty());
        vis.set_if_neq(if empty { Visibility::Inherited } else { Visibility::Hidden });
    }
}

/// The edit of a property: [`TabAction::SetValue`] as an [`Action`].
pub fn set_action(target: &PropKey, value: PropValue, merge: bool) -> Action {
    Action::Tab(TabAction::SetValue { key: target.clone(), value, merge })
}

fn select(
    ev: On<ValueChange<Entity>>,
    fields: Query<&PropField>,
    idx: Query<&OptionIndex>,
    mut actions: ResMut<Actions>,
) {
    let (Ok(f), Ok(i)) = (fields.get(ev.source), idx.get(ev.value)) else { return };
    let value = match f.values.get(i.0) {
        Some(v) => v.clone(),
        None => f.options.get(i.0).cloned().unwrap_or_default(),
    };
    actions.push(set_action(&f.target, PropValue::Str(value), false));
}

fn text_changed(
    ev: On<TextEditChange>,
    mut fields: Query<(&PropField, &EditableText, &mut FieldText)>,
    mut actions: ResMut<Actions>,
) {
    let Ok((f, t, mut sync)) = fields.get_mut(ev.event_target()) else { return };
    let value = t.value().to_string();
    if !sync.ready {
        sync.ready = value == sync.last;
        return;
    }
    if value == sync.last {
        return;
    }
    sync.last = value.clone();
    actions.push(set_action(&f.target, PropValue::Str(value), true));
}

fn set_component(f: &PropField, comp: usize, value: f32, is_final: bool) -> Action {
    Action::Tab(TabAction::SetComponent { key: f.target.clone(), comp, value, is_final })
}

/// [`FieldEditEnded`] after a final edit of `source`.
fn ended(commands: &mut Commands, source: Entity, is_final: bool) {
    if is_final {
        commands.trigger(FieldEditEnded { source });
    }
}

fn number_f32(
    ev: On<ValueChange<f32>>,
    fields: Query<&PropField>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    if let Ok(f) = fields.get(ev.source) {
        actions.push(set_component(f, f.comp, ev.value, ev.is_final));
        ended(&mut commands, ev.source, ev.is_final);
    }
}

fn number_i32(
    ev: On<ValueChange<i32>>,
    fields: Query<&PropField>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    if let Ok(f) = fields.get(ev.source) {
        actions.push(set_component(f, 0, ev.value as f32, ev.is_final));
        ended(&mut commands, ev.source, ev.is_final);
    }
}

/// A color picker: writes all components as linear values (one merged undo step per drag).
fn color(ev: On<ValueChange<Color>>, fields: Query<&PropField>, mut actions: ResMut<Actions>, mut commands: Commands) {
    let Ok(f) = fields.get(ev.source) else { return };
    let c = ev.value.to_linear();
    for (comp, value) in [c.red, c.green, c.blue, c.alpha].into_iter().enumerate() {
        actions.push(set_component(f, comp, value, false));
    }
    ended(&mut commands, ev.source, ev.is_final);
}

fn checkbox_changed(
    ev: On<ValueChange<bool>>,
    fields: Query<&PropField>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    if let Ok(f) = fields.get(ev.source) {
        if ev.value {
            commands.entity(ev.source).insert(Checked);
        } else {
            commands.entity(ev.source).remove::<Checked>();
        }
        actions.push(set_action(&f.target, PropValue::Bool(ev.value), false));
        ended(&mut commands, ev.source, true);
    }
}
