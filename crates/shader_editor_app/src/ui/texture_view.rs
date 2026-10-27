//! Texture asset tab (design §4.4): the settings form on the left (path, loader settings, sampler), the preview on
//! the right ([`super::image_preview`], shared with the image tab), decoded with the document's current (unsaved)
//! settings. Easy mode (decision 0028) is only the use-case presets in the editor topbar, without the form; one of
//! them is always selected ("Default": no sampler stored, design §6.3), and a sampler matching none keeps the tab in
//! Advanced, the form with every field. Built-in textures (`@internal/…`) show their values as text.
//!
//! Form widgets are the Properties ones ([`PropField`] + `PropTarget::Texture`), so edits take the usual `SetValue` /
//! `SetComponent` path; the path field applies on Enter (`TextureAction::SetPath`, refused when another Texture
//! asset points at the file). The form is rebuilt only when [`TextureDoc::layout`] changes.

use bevy::feathers::controls::{FeathersTextInput, FeathersTextInputContainer};
use bevy::input::keyboard::KeyboardInput;
use bevy::input_focus::FocusedInput;
use bevy::prelude::*;
use bevy::text::EditableText;
use shader_graph_core::Value;

use super::dialog::InitText;
use super::image_preview::{preview_area, preview_controls};
use super::props::{checkbox_row, checkbox_w, integer, labeled, number, select_w};
use super::widgets::{Caption, GroupButton, button_group, subpane_box};
use super::{TEXT, TEXT_DIM, VIEW_BG, ViewOf, scroll_column, text};
use crate::game_image::{GameImageCache, previewable};
use crate::state::{Action, Actions, Editor, PropTarget, TextureAction};
use crate::tab::ViewKind;
use crate::texture_doc::{FilterPreset, TextureDoc, TextureField, fits_easy};

/// Width of the form column.
const FORM_W: f32 = 390.0;
const WARN: Color = Color::srgb(0.95, 0.78, 0.35);

/// The editor topbar row of a texture tab (Easy / Advanced, presets, then the preview's controls), rebuilt with the
/// form.
#[derive(Component, Default, Clone)]
pub(crate) struct TextureViewBar;

/// Settings form, rebuilt on [`TextureDoc::layout`].
#[derive(Component, Default, Clone)]
pub struct TextureViewForm;

/// The path text field (applies on Enter).
#[derive(Component, Default, Clone)]
pub struct TexturePathField;

pub struct TextureViewPlugin;

impl Plugin for TextureViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (redraw_bar, redraw_form).after(crate::apply_actions)).add_observer(path_on_enter);
    }
}

pub fn texture_view() -> impl Scene {
    bsn! {
        ViewOf(ViewKind::Texture)
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            flex_direction: FlexDirection::Column,
        }
        BackgroundColor(VIEW_BG)
        Visibility::Hidden
        Children [
            Node { flex_grow: 1.0, min_height: px(0), flex_direction: FlexDirection::Row }
            Children [
                @scroll_column()
                TextureViewForm
                Node { width: px(FORM_W), flex_grow: 0.0, flex_shrink: 0.0, padding: UiRect::new(px(12), px(12), px(8), px(24)), row_gap: px(8) }
                --
                @preview_area(ViewKind::Texture)
            ]
        ]
    }
}

/// A field's value as text (read-only built-in textures).
fn value_text(v: &Value) -> String {
    match v {
        Value::Bool(b) => if *b { "on" } else { "off" }.to_string(),
        Value::Float(f) => format!("{f}"),
        Value::Int(i) => i.to_string(),
        other => other.as_str().to_string(),
    }
}

/// The widget editing `field` (or its value as text on a read-only document).
fn field_widget(t: &TextureDoc, field: TextureField) -> Box<dyn Scene> {
    let v = field.get(&t.asset.settings);
    if t.is_internal() {
        return Box::new(text(value_text(&v), TEXT, 13.0));
    }
    let target = PropTarget::Texture(field);
    match &v {
        Value::Bool(b) => checkbox_w(target, *b),
        Value::Float(f) => number(target, 0, *f, None),
        Value::Int(i) => integer(target, *i),
        _ => {
            let options: Vec<String> = field.options().into_iter().map(str::to_string).collect();
            let cur = options.iter().position(|o| o == v.as_str()).unwrap_or(0);
            select_w(target, options, Vec::new(), cur)
        }
    }
}

/// A labelled row editing `field`; an editable bool is a captioned checkbox.
fn field_row(t: &TextureDoc, field: TextureField) -> Box<dyn Scene> {
    match field.get(&t.asset.settings) {
        Value::Bool(b) if !t.is_internal() => checkbox_row(field.label(), PropTarget::Texture(field), b),
        _ => Box::new(labeled(field.label().into(), field_widget(t, field))),
    }
}

/// A note under a [`labeled`] row, in its widget column.
fn hint(s: impl Into<String>) -> Box<dyn Scene> {
    hint_at(s, 116.0)
}

/// A note under a [`checkbox_row`], lined up with the checkbox's caption (Feathers: an 18 px box, a 4 px gap).
fn checkbox_hint(s: impl Into<String>) -> Box<dyn Scene> {
    hint_at(s, 22.0)
}

fn hint_at(s: impl Into<String>, indent: f32) -> Box<dyn Scene> {
    let s = s.into();
    Box::new(bsn! { Node { margin: UiRect::left(px(indent)) } Children [ @text(s, TEXT_DIM, 11.0) ] })
}

/// The form's subpanes: Image (the file), Loader, Sampler, then the preview's notes.
fn form(t: &TextureDoc, clamp_to_border: bool) -> Vec<Box<dyn Scene>> {
    let mut image: Vec<Box<dyn Scene>> = Vec::new();
    if t.is_internal() {
        image.push(Box::new(labeled("Path".into(), Box::new(text(format!("assets/{}", t.asset.path), TEXT, 13.0)))));
        image.push(hint("Built-in texture: copied there on every regeneration; read-only."));
    } else {
        let init = InitText(t.asset.path.clone());
        let field: Box<dyn Scene> = Box::new(bsn! {
            @FeathersTextInputContainer
            Node { flex_grow: 1.0 }
            Children [ @FeathersTextInput TexturePathField init ]
        });
        image.push(Box::new(labeled("Path".into(), field)));
        image.push(hint("Under the game's assets/ dir (ktx2, png); Enter applies. One file - one Texture asset."));
    }
    // Hints only where something can be changed.
    let editable = !t.is_internal();
    let mut loader = vec![field_row(t, TextureField::IsSrgb)];
    if editable {
        loader.push(checkbox_hint("Rebuild needed in the game (not hot-reloaded)."));
    }
    loader.push(field_row(t, TextureField::ExpandGrayscale));
    if editable {
        loader.push(checkbox_hint("Rebuild needed in the game. Not previewed: the editor always expands grayscale."));
    }
    let mut sampler = vec![field_row(t, TextureField::DefaultSampler)];
    if t.asset.settings.sampler.is_none() {
        let note = "The game's ImagePlugin default: with Bevy's ImagePlugin::default() Linear filters, Clamp to edge \
                    (not Repeat). Turning it off starts from these values.";
        // A built-in texture shows the bool as text in a labelled row.
        sampler.push(if editable { checkbox_hint(note) } else { hint(note) });
    } else {
        sampler.extend(TextureField::SAMPLER.into_iter().map(|f| field_row(t, f)));
    }
    let mut out = vec![subpane_box("Image", image), subpane_box("Loader", loader), subpane_box("Sampler", sampler)];
    for note in previewable(&t.asset.settings, clamp_to_border).1 {
        out.push(Box::new(text(note, WARN, 12.0)));
    }
    out
}

/// Rebuilds the editor topbar when another Texture tab is shown, its form is rebuilt, or the settings start or stop
/// fitting a preset (a dragged number field keeps the form but must disable Easy).
fn redraw_bar(
    editor: Res<Editor>,
    bar: Query<Entity, With<TextureViewBar>>,
    mut last: Local<Option<(u64, bool)>>,
    mut commands: Commands,
) {
    let Some(t) = editor.texture_doc() else { return };
    // Easy shows only the presets: a sampler matching none of them stays in Advanced (design §6.3).
    let fits = fits_easy(&t.asset.settings);
    let key = (t.layout, fits);
    if *last == Some(key) {
        return;
    }
    let Ok(bar) = bar.single() else { return };
    *last = Some(key);
    commands.entity(bar).despawn_children();
    let mut controls: Vec<Box<dyn Scene>> = Vec::new();
    let item = |label: &str, action: Action, selected: bool, tip: String| GroupButton {
        caption: Caption::Text(label.into()),
        action,
        selected,
        disabled: false,
        tip,
    };
    if !t.is_internal() {
        let modes = [(true, "Easy", "Pick what the texture is for"), (false, "Advanced", "Every setting in a form")]
            .map(|(easy, label, tip)| {
                let refused = easy && !fits;
                let tip = if refused { "Settings match no preset - edit them in Advanced" } else { tip };
                let action = Action::Texture(TextureAction::SetEasy(easy));
                GroupButton { disabled: refused, ..item(label, action, t.easy == easy, tip.into()) }
            });
        controls.push(Box::new(button_group(modes.into())));
        if t.easy {
            let current = FilterPreset::of(&t.asset.settings);
            let presets = FilterPreset::ALL.map(|p| {
                item(p.label(), Action::Texture(TextureAction::Preset(p)), current == Some(p), p.tip().into())
            });
            controls.push(Box::new(bsn! { @text("Use for", TEXT_DIM, 12.0) Node { margin: UiRect::left(px(10)) } }));
            controls.push(Box::new(button_group(presets.into())));
        }
    }
    controls.push(Box::new(bsn! { Node { flex_grow: 1.0 } }));
    controls.extend(preview_controls(t));
    for c in controls {
        commands.spawn_scene(bsn! { @{c} ChildOf(bar) });
    }
}

/// Rebuilds the form when another Texture tab is shown or its form must follow the model.
fn redraw_form(
    editor: Res<Editor>,
    cache: Res<GameImageCache>,
    mut form_body: Query<(Entity, &mut Node), With<TextureViewForm>>,
    mut last: Local<Option<(u64, bool)>>,
    mut commands: Commands,
) {
    let Some(t) = editor.texture_doc() else { return };
    let key = (t.layout, cache.clamp_to_border);
    if *last == Some(key) {
        return;
    }
    let Ok((body, mut node)) = form_body.single_mut() else { return };
    *last = Some(key);
    // Easy mode has no form: the presets are the whole choice.
    let display = if t.easy && !t.is_internal() { Display::None } else { Display::Flex };
    if node.display != display {
        node.display = display;
    }
    commands.entity(body).despawn_children();
    for r in form(t, cache.clamp_to_border) {
        commands.spawn_scene(bsn! { @{r} ChildOf(body) });
    }
}

/// Enter in the path field points the asset at the typed file.
fn path_on_enter(
    ev: On<FocusedInput<KeyboardInput>>,
    fields: Query<&EditableText, With<TexturePathField>>,
    mut actions: ResMut<Actions>,
) {
    if ev.input.key_code == KeyCode::Enter
        && ev.input.state.is_pressed()
        && let Ok(t) = fields.get(ev.focused_entity)
    {
        actions.push(Action::Texture(TextureAction::SetPath(t.value().to_string())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_text_reads_fields() {
        assert_eq!(value_text(&Value::Bool(true)), "on");
        assert_eq!(value_text(&Value::Int(4)), "4");
        assert_eq!(value_text(&Value::Str("Linear".into())), "Linear");
    }
}
