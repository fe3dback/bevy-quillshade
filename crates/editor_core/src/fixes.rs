//! Feathers glue copied from bevy_noodle's `examples/feathers_fixes` (still needed on Bevy 0.20):
//! visible text cursors for late-spawned fields, focusable text-field frames inside nodes, and
//! number fields that show what they send.

use bevy::feathers::controls::{
    ColorInputValue, ColorSwatchValue, FeathersCheckbox, FeathersColorInput, FeathersNumberInput, FeathersTextInput,
    FeathersTextInputContainer, NumberInputValue,
};
use bevy::feathers::theme::UiTheme;
use bevy::feathers::tokens;
use bevy::input_focus::tab_navigation::TabIndex;
use bevy::input_focus::{FocusCause, FocusGained, InputFocus};
use bevy::prelude::*;
use bevy::text::{EditableText, TextCursorStyle};
use bevy::ui::Checked;
use bevy::ui_widgets::ValueChange;

pub struct FeathersFixesPlugin;

impl Plugin for FeathersFixesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreUpdate, (color_new_cursors, focusable_frames))
            .add_systems(Last, (init_late_checkboxes, init_late_color_inputs))
            .add_observer(focus_field_in_frame)
            .add_observer(show_number(NumberInputValue::F32))
            .add_observer(show_number(NumberInputValue::I32));
    }
}

/// `FeathersCheckbox` spawns its checkmark with `Visibility::Inherited` in `SpawnScene` and only hides it
/// in `PreUpdate` on the next frame, causing unchecked checkboxes to flash checked for 1 frame.
fn init_late_checkboxes(
    checkboxes: Query<(Has<Checked>, &Children), Added<FeathersCheckbox>>,
    q_children: Query<&Children>,
    mut q_vis: Query<&mut Visibility>,
) {
    for (checked, children) in &checkboxes {
        if let Some(&outline) = children.first()
            && let Ok(outline_children) = q_children.get(outline)
            && let Some(&mark) = outline_children.first()
            && let Ok(mut vis) = q_vis.get_mut(mark)
        {
            *vis = if checked { Visibility::Inherited } else { Visibility::Hidden };
        }
    }
}

/// `FeathersColorInput` syncs `ColorInputValue -> ColorSwatchValue -> BackgroundColor` across two deferred
/// `PostUpdate` steps, causing newly spawned color swatches to flash `Color::WHITE` on their first frame.
fn init_late_color_inputs(
    inputs: Query<(&ColorInputValue, &Children), (With<FeathersColorInput>, Changed<ColorInputValue>)>,
    mut q_swatches: Query<(&mut ColorSwatchValue, &Children)>,
    mut q_bgs: Query<&mut BackgroundColor>,
) {
    for (value, children) in &inputs {
        for &child in children {
            if let Ok((mut swatch_val, swatch_children)) = q_swatches.get_mut(child) {
                swatch_val.0 = value.0;
                if let Some(&fg0) = swatch_children.first()
                    && let Ok(mut bg) = q_bgs.get_mut(fg0)
                {
                    bg.0 = value.0;
                }
                if let Some(&fg1) = swatch_children.get(1)
                    && let Ok(mut bg) = q_bgs.get_mut(fg1)
                {
                    bg.0 = value.0.with_alpha(1.0);
                }
            }
        }
    }
}

/// Feathers colors text cursors only when the theme changes, so fields
/// spawned later keep a dark default cursor that is hard to see.
fn color_new_cursors(mut cursors: Query<&mut TextCursorStyle, Added<FeathersTextInput>>, theme: Res<UiTheme>) {
    for mut cursor in &mut cursors {
        cursor.color = theme.color(&tokens::TEXT_INPUT_CURSOR);
        cursor.selection_color = theme.color(&tokens::TEXT_INPUT_SELECTION);
        cursor.unfocused_selection_color = theme.color(&tokens::TEXT_INPUT_SELECTION_UNFOCUSED);
    }
}

/// A press on the few pixels of frame around a field focuses nothing, and
/// lands on the node: it selects or drags it. A negative `TabIndex` makes the
/// frame a control (focusable by pointer, skipped by Tab)…
fn focusable_frames(
    frames: Query<Entity, (Added<FeathersTextInputContainer>, Without<TabIndex>)>,
    mut commands: Commands,
) {
    for frame in &frames {
        commands.entity(frame).try_insert(TabIndex(-1));
    }
}

/// …that hands its focus to the field inside.
fn focus_field_in_frame(
    gained: On<FocusGained>,
    frames: Query<&Children, With<FeathersTextInputContainer>>,
    fields: Query<(), With<EditableText>>,
    mut focus: ResMut<InputFocus>,
) {
    // Only the frame's own focus, not a field's bubbling up.
    if gained.entity != gained.original_event_target() {
        return;
    }
    if let Ok(children) = frames.get(gained.entity)
        && let Some(field) = children.iter().find(|&c| fields.contains(c))
    {
        focus.set(field, FocusCause::Pressed);
    }
}

/// Feathers number fields are controlled: they show the value they are
/// given, not what is typed or dragged. The examples keep the value in their
/// own components, so each field shows what it just sent.
fn show_number<T: Copy + Send + Sync + 'static>(
    value: fn(T) -> NumberInputValue,
) -> impl Fn(On<ValueChange<T>>, Query<(), With<FeathersNumberInput>>, Commands) {
    move |change, fields, mut commands| {
        if fields.contains(change.source) {
            commands.entity(change.source).try_insert(value(change.value));
        }
    }
}
