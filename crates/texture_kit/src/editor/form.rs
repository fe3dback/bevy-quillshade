//! `D`'s Texture editor kind (design §4.4, decision 0031): the settings form on the left of the editor content (path,
//! loader settings, sampler), the preview on the right ([`super::preview`], shared with the image tab), decoded with
//! the document's current (unsaved) settings; the editor topbar and the right sidebar (what the texture is, its
//! image, its problems, the hotkeys).
//!
//! Easy mode (decision 0028, only with [`crate::TexturePolicy::presets`]) is only the use-case presets in the editor
//! topbar, without the form; one of them is always selected ("Default": no sampler stored, design §6.3), and a
//! sampler matching none keeps the tab in Advanced, the form with every field. A domain without presets always shows
//! the form (design §7.2). Built-in textures (`@internal/…`) show their values as text.
//!
//! Form widgets are the property widgets ([`editor_core::ui::props`] + `TextureProp::Field`, keyed by domain), so edits
//! take the usual `SetValue` / `SetComponent` path; the path field applies on Enter (`TextureAction::SetPath`, refused when another
//! Texture asset points at the file). The form is rebuilt only when [`TextureDoc::layout`] changes.

use std::marker::PhantomData;

use bevy::feathers::controls::{FeathersTextInput, FeathersTextInputContainer};
use bevy::input::keyboard::KeyboardInput;
use bevy::input_focus::FocusedInput;
use bevy::prelude::*;
use bevy::text::EditableText;
use editor_core::action::{Action, Actions};
use editor_core::doc_kind::TabDoc;
use editor_core::dynval::PropValue;
use editor_core::editors::{EditorAreas, SidebarKey, ViewOf, bar_row, fill_sidebar_with, sidebar_body};
use editor_core::state::editor::Editor;
use editor_core::tab::ViewKindId;
use editor_core::theme::{TEXT, TEXT_DIM, text};
use editor_core::ui::props::{InitText, checkbox_row, checkbox_w, hotkey_row, integer, labeled, number, select_values};
use editor_core::ui::shell::VIEW_BG;
use editor_core::ui::widgets::{Caption, GroupButton, button_group, scroll_column, subpane_box};

use super::{
    FilterPreset, GameImageCache, PREVIEW_HOTKEYS, PreviewInfoText, TextureAction, TextureDoc, TextureDomain,
    TextureEdit, TextureField, TextureProp, active, fits_easy, preview_area, preview_controls, previewable,
    texture_key,
};

/// Width of the form column.
const FORM_W: f32 = 390.0;
const WARN: Color = Color::srgb(0.95, 0.78, 0.35);
const ERROR: Color = Color::srgb(0.95, 0.42, 0.42);

/// `D`'s Texture editor kind.
pub struct TextureEditor<D>(PhantomData<fn() -> D>);

marker! {
    /// The editor topbar row of `D`'s Texture tab (Easy / Advanced, presets, then the preview's controls), rebuilt
    /// with the form.
    TextureViewBar
}

marker! {
    /// `D`'s settings form, rebuilt on [`TextureDoc::layout`].
    TextureViewForm
}

marker! {
    /// The path text field of `D`'s form (applies on Enter).
    TexturePathField
}

marker! {
    /// The right sidebar body of `D`'s Texture editor.
    TextureSidebarBody
}

impl<D: TextureDomain> EditorAreas for TextureEditor<D> {
    const VIEW: ViewKindId = D::VIEW;
    const SIDEBAR_TITLE: &'static str = "Texture";

    /// The bar row, filled by [`redraw_bar`].
    fn topbar() -> Option<impl Scene> {
        let bar = TextureViewBar::<D>::default();
        Some(bsn! { @bar_row() bar })
    }

    /// What the texture is, its image (`PreviewInfoText`, filled by the preview), its problems, the hotkeys.
    fn sidebar() -> Option<impl Scene> {
        let body = TextureSidebarBody::<D>::default();
        Some(bsn! { @sidebar_body() body })
    }

    fn build(app: &mut App) {
        app.add_systems(Update, redraw_sidebar::<D>);
    }
}

fn redraw_sidebar<D: TextureDomain>(
    editor: Res<Editor>,
    body: Query<Entity, With<TextureSidebarBody<D>>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some(t) = active::<D>(&editor) else { return key.forget() };
    fill_sidebar_with(&mut commands, &body, &mut key, SidebarKey::now(&editor), || texture_rows(&editor, t));
}

/// Hotkeys of the Texture tab besides the preview's.
const TEXTURE_HOTKEYS: [(&str, &str); 2] = [("Ctrl+Z / Y", "Undo / redo"), ("Ctrl+S", "Save and regenerate")];

/// A Texture tab (design §4.4): what it is, its image, its problems ([`TextureDomain::problems`]), the hotkeys (the
/// form and preview are the view).
fn texture_rows<D: TextureDomain>(editor: &Editor, t: &TextureDoc<D>) -> Vec<Box<dyn Scene>> {
    let mut texture: Vec<Box<dyn Scene>> = Vec::new();
    texture.push(Box::new(text(t.base_title(), TEXT, 15.0)));
    let kind = if t.is_internal() { "Built-in texture, read-only" } else { "Type: Texture asset" };
    texture.push(Box::new(text(kind, TEXT_DIM, 12.0)));
    texture.push(Box::new(text(format!("Image: assets/{}", t.asset.path), TEXT_DIM, 12.0)));
    let info = PreviewInfoText::<D>::default();
    texture.push(Box::new(bsn! { @text("", TEXT_DIM, 12.0) info }));
    if !t.is_internal() {
        texture.push(Box::new(text(D::SETTINGS_NOTE, TEXT_DIM, 12.0)));
    }
    let mut out = vec![subpane_box("Texture", texture)];
    let problems = if t.is_internal() { Vec::new() } else { D::problems(editor, &t.aref) };
    if !problems.is_empty() {
        let rows = problems.iter().map(|diag| -> Box<dyn Scene> {
            let c = if diag.is_error() { ERROR } else { WARN };
            Box::new(text(format!("{}: {}", diag.code, diag.message), c, 12.0))
        });
        out.push(subpane_box("Problems", rows.collect()));
    }
    let edits = if t.is_internal() { &[][..] } else { &TEXTURE_HOTKEYS[..] };
    let rows =
        PREVIEW_HOTKEYS.iter().chain(edits).map(|&(keys, what)| -> Box<dyn Scene> { Box::new(hotkey_row(keys, what)) });
    out.push(subpane_box("Hotkeys", rows.collect()));
    out
}

/// `D`'s Texture tab's window systems: its editor topbar, the form and the path field.
pub struct TextureViewPlugin<D>(PhantomData<fn() -> D>);

impl<D> Default for TextureViewPlugin<D> {
    fn default() -> Self {
        TextureViewPlugin(PhantomData)
    }
}

impl<D: TextureDomain> Plugin for TextureViewPlugin<D> {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (redraw_bar::<D>, redraw_form::<D>).after(editor_core::apply_actions))
            .add_observer(path_on_enter::<D>);
    }
}

/// `D`'s Texture view in the editor content.
pub fn texture_view<D: TextureDomain>() -> impl Scene {
    let (view, form) = (ViewOf(D::VIEW), TextureViewForm::<D>::default());
    let area: Box<dyn Scene> = Box::new(preview_area::<D>(D::VIEW));
    bsn! {
        view
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
                form
                Node { width: px(FORM_W), flex_grow: 0.0, flex_shrink: 0.0, padding: UiRect::new(px(12), px(12), px(8), px(24)), row_gap: px(8) }
                --
                @{area}
            ]
        ]
    }
}

/// A field's value as text (read-only built-in textures).
fn value_text(v: &PropValue) -> String {
    match v {
        PropValue::Bool(b) => if *b { "on" } else { "off" }.to_string(),
        PropValue::Float(f) => format!("{f}"),
        PropValue::Int(i) => i.to_string(),
        other => other.as_str().to_string(),
    }
}

/// The widget editing `field` (or its value as text on a read-only document).
fn field_widget<D: TextureDomain>(t: &TextureDoc<D>, field: TextureField) -> Box<dyn Scene> {
    let v = field.get(&t.asset.settings);
    if t.is_internal() {
        return Box::new(text(value_text(&v), TEXT, 13.0));
    }
    let target = texture_key::<D>(TextureProp::Field(field));
    match &v {
        PropValue::Bool(b) => checkbox_w(target, *b),
        PropValue::Float(f) => number(target, 0, *f, None),
        PropValue::Int(i) => integer(target, *i),
        _ => {
            let options: Vec<String> = field.options().into_iter().map(str::to_string).collect();
            let cur = options.iter().position(|o| o == v.as_str()).unwrap_or(0);
            select_values(target, options, Vec::new(), cur)
        }
    }
}

/// A labelled row editing `field`; an editable bool is a captioned checkbox.
fn field_row<D: TextureDomain>(t: &TextureDoc<D>, field: TextureField) -> Box<dyn Scene> {
    match field.get(&t.asset.settings) {
        PropValue::Bool(b) if !t.is_internal() => {
            checkbox_row(field.label(), texture_key::<D>(TextureProp::Field(field)), b)
        }
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
fn form<D: TextureDomain>(t: &TextureDoc<D>, clamp_to_border: bool) -> Vec<Box<dyn Scene>> {
    let mut image: Vec<Box<dyn Scene>> = Vec::new();
    if t.is_internal() {
        image.push(Box::new(labeled("Path".into(), Box::new(text(format!("assets/{}", t.asset.path), TEXT, 13.0)))));
        image.push(hint("Built-in texture: copied there on every regeneration; read-only."));
    } else {
        let (init, path) = (InitText(t.asset.path.clone()), TexturePathField::<D>::default());
        let field: Box<dyn Scene> = Box::new(bsn! {
            @FeathersTextInputContainer
            Node { flex_grow: 1.0 }
            Children [ @FeathersTextInput path init ]
        });
        image.push(Box::new(labeled("Path".into(), field)));
        let formats = D::POLICY.extensions.join(", ");
        image.push(hint(format!(
            "Under the game's assets/ dir ({formats}); Enter applies. One file - one Texture asset."
        )));
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
fn redraw_bar<D: TextureDomain>(
    editor: Res<Editor>,
    bar: Query<Entity, With<TextureViewBar<D>>>,
    mut last: Local<Option<(u64, bool)>>,
    mut commands: Commands,
) {
    let Some(t) = active::<D>(&editor) else { return };
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
    // A domain without presets has no Easy mode: the form is the whole tab (design §7.2).
    if !t.is_internal() && D::POLICY.presets {
        let modes = [(true, "Easy", "Pick what the texture is for"), (false, "Advanced", "Every setting in a form")]
            .map(|(easy, label, tip)| {
                let refused = easy && !fits;
                let tip = if refused { "Settings match no preset - edit them in Advanced" } else { tip };
                let action = Action::from(TextureEdit::<D>::new(TextureAction::SetEasy(easy)));
                GroupButton { disabled: refused, ..item(label, action, t.easy == easy, tip.into()) }
            });
        controls.push(Box::new(button_group(modes.into())));
        if t.easy {
            let current = FilterPreset::of(&t.asset.settings);
            let presets = FilterPreset::ALL.map(|p| {
                item(
                    p.label(),
                    Action::from(TextureEdit::<D>::new(TextureAction::Preset(p))),
                    current == Some(p),
                    p.tip().into(),
                )
            });
            controls.push(Box::new(bsn! { @text("Use for", TEXT_DIM, 12.0) Node { margin: UiRect::left(px(10)) } }));
            controls.push(Box::new(button_group(presets.into())));
        }
    }
    controls.push(Box::new(bsn! { Node { flex_grow: 1.0 } }));
    controls.extend(preview_controls::<D>(t));
    controls.extend(D::topbar_extras(t));
    for c in controls {
        commands.spawn_scene(bsn! { @{c} ChildOf(bar) });
    }
}

/// Rebuilds the form when another Texture tab is shown or its form must follow the model.
fn redraw_form<D: TextureDomain>(
    editor: Res<Editor>,
    cache: Res<GameImageCache<D>>,
    mut form_body: Query<(Entity, &mut Node), With<TextureViewForm<D>>>,
    mut last: Local<Option<(u64, bool)>>,
    mut commands: Commands,
) {
    let Some(t) = active::<D>(&editor) else { return };
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
fn path_on_enter<D: TextureDomain>(
    ev: On<FocusedInput<KeyboardInput>>,
    fields: Query<&EditableText, With<TexturePathField<D>>>,
    mut actions: ResMut<Actions>,
) {
    if ev.input.key_code == KeyCode::Enter
        && ev.input.state.is_pressed()
        && let Ok(t) = fields.get(ev.focused_entity)
    {
        actions.push(Action::from(TextureEdit::<D>::new(TextureAction::SetPath(t.value().to_string()))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_text_reads_fields() {
        assert_eq!(value_text(&PropValue::Bool(true)), "on");
        assert_eq!(value_text(&PropValue::Int(4)), "4");
        assert_eq!(value_text(&PropValue::Str("Linear".into())), "Linear");
    }
}
