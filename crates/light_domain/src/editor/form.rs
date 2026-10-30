//! The environment editor kind (design §9.2): the form in the editor content (a cube-map picker over the domain's
//! Light Texture assets plus "None", the skybox brightness and the environment intensity), an editor topbar opening
//! the cube map, and a right sidebar with what the environment is, its problems and the hotkeys. Every field is a
//! core property widget keyed by [`EnvironmentProp`].

use bevy::prelude::*;
use editor_core::doc_kind::TabDoc;
use editor_core::editors::{EditorAreas, SidebarKey, ViewOf, bar_row, fill_sidebar_with, sidebar_body};
use editor_core::state::editor::Editor;
use editor_core::tab::ViewKindId;
use editor_core::theme::{TEXT, TEXT_DIM, text};
use editor_core::ui::icon_tool_button;
use editor_core::ui::props::{hotkey_row, labeled, number_ranged, select_values};
use editor_core::ui::shell::VIEW_BG;
use editor_core::ui::widgets::{scroll_column, subpane_box};
use texture_kit::editor::TEXTURE_ICON;

use super::{ENVIRONMENT_VIEW, EnvironmentDoc, EnvironmentProp, LightState};
use crate::backend::{LIGHT, TEXTURE_SUFFIX, texture_ref};

/// Widest the form grows in the editor content (logical px).
const FORM_W: f32 = 560.0;
const WARN: Color = Color::srgb(0.95, 0.78, 0.35);
const ERROR: Color = Color::srgb(0.95, 0.42, 0.42);

/// Soft limits of the number fields: dragging stays inside, a typed value may leave them.
const BRIGHTNESS_RANGE: (f32, f32) = (0.0, 100_000.0);
const INTENSITY_RANGE: (f32, f32) = (0.0, 10.0);

/// The environment editor kind.
pub struct EnvironmentEditor;

/// The editor topbar row of the environment tab (opening the cube map).
#[derive(Component, Default, Clone)]
pub struct EnvironmentViewBar;

/// The form column in the editor content, rebuilt on [`EnvironmentDoc::layout`].
#[derive(Component, Default, Clone)]
pub struct EnvironmentForm;

/// The right sidebar body of the environment editor.
#[derive(Component, Default, Clone)]
pub struct EnvironmentSidebarBody;

impl EditorAreas for EnvironmentEditor {
    const VIEW: ViewKindId = ENVIRONMENT_VIEW;
    const SIDEBAR_TITLE: &'static str = "Environment";

    fn topbar() -> Option<impl Scene> {
        Some(bsn! { @bar_row() EnvironmentViewBar })
    }

    fn sidebar() -> Option<impl Scene> {
        Some(bsn! { @sidebar_body() EnvironmentSidebarBody })
    }

    fn build(app: &mut App) {
        app.add_systems(Update, (redraw_bar, redraw_form, redraw_sidebar).after(editor_core::apply_actions));
    }
}

/// The active environment tab with its index.
fn active(editor: &Editor) -> Option<(usize, &EnvironmentDoc)> {
    let i = editor.active?;
    Some((i, editor.tabs.get(i)?.get::<EnvironmentDoc>()?))
}

/// The environment view in the editor content.
pub fn environment_view() -> impl Scene {
    bsn! {
        ViewOf(ENVIRONMENT_VIEW)
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
            // The bottom margin keeps the form clear of the file type watermark.
            @scroll_column()
            EnvironmentForm
            Node { padding: UiRect::new(px(16), px(16), px(12), px(56)), row_gap: px(12) }
        ]
    }
}

/// The domain's Light Texture assets (their paths), as of the last rescan.
fn light_textures(editor: &Editor) -> Vec<String> {
    let files = editor.kind_files.get(&LIGHT).into_iter().flatten();
    files.filter(|r| r.suffix == TEXTURE_SUFFIX).map(|r| r.path.clone()).collect()
}

/// The picker's labels and values: "None" (`""`), then every Light texture; a cube map that is none of them is
/// listed as missing, so the picker shows what the file says. The index of the current one.
pub(super) fn cubemap_options(textures: &[String], current: Option<&str>) -> (Vec<String>, Vec<String>, usize) {
    let (mut labels, mut values) = (vec!["None".to_string()], vec![String::new()]);
    for t in textures {
        labels.push(t.clone());
        values.push(t.clone());
    }
    if let Some(c) = current.filter(|c| !textures.iter().any(|t| t == c)) {
        labels.push(format!("{c} (missing)"));
        values.push(c.to_string());
    }
    let at = values.iter().position(|v| v == current.unwrap_or_default()).unwrap_or(0);
    (labels, values, at)
}

/// The form: one section with the three fields and what each means.
fn form(doc: &EnvironmentDoc, textures: &[String]) -> Vec<Box<dyn Scene>> {
    let (labels, values, at) = cubemap_options(textures, doc.env.cubemap.as_deref());
    let picker = select_values(EnvironmentProp::Cubemap, labels, values, at);
    let brightness =
        number_ranged(EnvironmentProp::SkyboxBrightness, 0, doc.env.skybox_brightness, Some(BRIGHTNESS_RANGE), None);
    let intensity = number_ranged(EnvironmentProp::EnvIntensity, 0, doc.env.env_intensity, Some(INTENSITY_RANGE), None);
    let rows: Vec<Box<dyn Scene>> = vec![
        Box::new(labeled("Cube map".into(), picker)),
        hint(
            "A Light Texture asset (.ktx2 cube map): Assets > New Texture..., or Create Texture Asset on a .ktx2 game \
             file.",
        ),
        Box::new(labeled("Sky brightness".into(), brightness)),
        hint("Brightness of the skybox in cd/m2 (Bevy's Skybox::brightness)."),
        Box::new(labeled("Env intensity".into(), intensity)),
        hint("Scale of the light the cube map casts (Bevy's EnvironmentMapLight::intensity)."),
    ];
    let pane = subpane_box("Environment", rows);
    vec![Box::new(bsn! {
        Node { width: percent(100), max_width: px(FORM_W), flex_direction: FlexDirection::Column }
        Children [ @{pane} ]
    })]
}

/// A note under a [`labeled`] row, in its widget column.
fn hint(s: &str) -> Box<dyn Scene> {
    let s = s.to_string();
    Box::new(bsn! { Node { margin: UiRect::left(px(116)) } Children [ @text(s, TEXT_DIM, 11.0) ] })
}

/// What the form shows: the project generation, the active tab, its [`EnvironmentDoc::layout`], the Light textures.
type FormKey = (u64, usize, u64, Vec<String>);

/// Rebuilds the form when another environment tab is shown, its form must follow the model (not during a drag) or
/// the Light textures changed.
fn redraw_form(
    editor: Res<Editor>,
    body: Query<Entity, With<EnvironmentForm>>,
    mut last: Local<Option<FormKey>>,
    mut commands: Commands,
) {
    let Some((i, doc)) = active(&editor) else {
        *last = None;
        return;
    };
    let textures = light_textures(&editor);
    let key = (editor.project_generation, i, doc.layout, textures);
    if last.as_ref() == Some(&key) {
        return;
    }
    let Ok(body) = body.single() else { return };
    commands.entity(body).despawn_children();
    for s in form(doc, &key.3) {
        commands.spawn_scene(bsn! { @{s} ChildOf(body) });
    }
    *last = Some(key);
}

/// Rebuilds the editor topbar when another environment tab is shown or its cube map changed: "Open cube map" while
/// it names a Light texture.
fn redraw_bar(
    editor: Res<Editor>,
    bar: Query<Entity, With<EnvironmentViewBar>>,
    mut last: Local<Option<(usize, Option<String>, bool)>>,
    mut commands: Commands,
) {
    let Some((i, doc)) = active(&editor) else {
        *last = None;
        return;
    };
    let cubemap = doc.env.cubemap.clone();
    let known = cubemap.as_ref().is_some_and(|c| light_textures(&editor).contains(c));
    let key = (i, cubemap, known);
    if last.as_ref() == Some(&key) {
        return;
    }
    let Ok(bar) = bar.single() else { return };
    commands.entity(bar).despawn_children();
    let item: Box<dyn Scene> = match &key.1 {
        Some(path) if known => Box::new(icon_tool_button(
            TEXTURE_ICON,
            "Open cube map",
            editor_core::action::Action::OpenAsset(texture_ref(path)),
            format!("Open the Light Texture asset {path}"),
        )),
        Some(path) => Box::new(text(format!("Cube map `{path}` is no Light Texture asset"), ERROR, 12.0)),
        None => Box::new(text("No cube map set", TEXT_DIM, 12.0)),
    };
    commands.spawn_scene(bsn! { @{item} ChildOf(bar) });
    *last = Some(key);
}

fn redraw_sidebar(
    editor: Res<Editor>,
    body: Query<Entity, With<EnvironmentSidebarBody>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some((_, doc)) = active(&editor) else { return key.forget() };
    fill_sidebar_with(&mut commands, &body, &mut key, SidebarKey::now(&editor), || sidebar_rows(&editor, doc));
}

/// Hotkeys of the environment tab.
const HOTKEYS: [(&str, &str); 2] = [("Ctrl+Z / Y", "Undo / redo"), ("Ctrl+S", "Save and regenerate")];

/// What the environment is, its problems (the Light domain's diagnostics of the asset), the hotkeys.
fn sidebar_rows(editor: &Editor, doc: &EnvironmentDoc) -> Vec<Box<dyn Scene>> {
    let about: Vec<Box<dyn Scene>> = vec![
        Box::new(text(doc.base_title(), TEXT, 15.0)),
        Box::new(text("Type: Environment", TEXT_DIM, 12.0)),
        Box::new(text(
            "The sky and environment light of a camera. Saving regenerates crates/quillshade/src/light/mod.rs, which \
             lists the environments without errors.",
            TEXT_DIM,
            12.0,
        )),
    ];
    let mut out = vec![subpane_box("Environment", about)];
    let rel = doc.aref.source_rel();
    let problems: Vec<Box<dyn Scene>> = editor
        .domains
        .get::<LightState>()
        .into_iter()
        .flat_map(|s| &s.diagnostics)
        .filter(|d| d.asset.as_deref() == Some(rel.as_str()))
        .map(|d| -> Box<dyn Scene> {
            let c = if d.is_error() { ERROR } else { WARN };
            Box::new(text(format!("{}: {}", d.code, d.message), c, 12.0))
        })
        .collect();
    if !problems.is_empty() {
        out.push(subpane_box("Problems", problems));
    }
    let hotkeys = HOTKEYS.map(|(keys, what)| -> Box<dyn Scene> { Box::new(hotkey_row(keys, what)) });
    out.push(subpane_box("Hotkeys", hotkeys.into()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_lists_none_the_textures_and_a_missing_one() {
        let textures = vec!["sky".to_string(), "skies/blue".to_string()];
        let (labels, values, at) = cubemap_options(&textures, None);
        assert_eq!(
            (labels, values, at),
            (
                vec!["None".into(), "sky".into(), "skies/blue".into()],
                vec![String::new(), "sky".into(), "skies/blue".into()],
                0
            )
        );
        assert_eq!(cubemap_options(&textures, Some("skies/blue")).2, 2);
        let (labels, values, at) = cubemap_options(&textures, Some("gone"));
        assert_eq!((labels[3].as_str(), values[3].as_str(), at), ("gone (missing)", "gone", 3));
    }
}
