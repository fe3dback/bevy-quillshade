//! Shader editor GUI (spec §9): Bevy UI + Feathers panels, bevy_noodle canvas, no egui.
//!
//! Data flow: widgets push [`state::Action`]s → [`apply_actions`] mutates [`state::Editor`]
//! (documents are the source of truth) → the canvas and panels redraw from it.

#![allow(clippy::type_complexity)] // Bevy system signatures

use std::path::PathBuf;

use bevy::feathers::FeathersPlugins;
use bevy::feathers::dark_theme::create_dark_theme;
use bevy::feathers::theme::UiTheme;
use bevy::prelude::*;
use bevy::window::{WindowResized, WindowResolution};
use bevy_noodle::prelude::*;

pub mod autolayout;
pub mod canvas;
pub mod config;
pub mod curve_doc;
mod demo;
pub mod doc;
mod external;
mod fixes;
pub mod highlight;
pub mod history;
pub mod inline;
pub mod search;
pub mod state;
pub mod tab;
pub mod ui;

use canvas::{CanvasPlugin, CanvasSystems, EditorCanvas};
use config::UserConfig;
use state::{Action, Actions, Editor};

/// Seconds of quiet before live validation runs.
const VALIDATE_DELAY: f32 = 0.25;

/// Project to open at startup (CLI argument, else the most recent one).
#[derive(Resource, Default)]
struct StartupProject(Option<PathBuf>);

/// Editor logic without windowing: state, actions, canvas sync. Used by the app and UI tests.
pub struct EditorCorePlugin;

impl Plugin for EditorCorePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Actions>().add_plugins(CanvasPlugin).add_systems(PreUpdate, attach_canvas).add_systems(
            Update,
            (apply_actions.before(CanvasSystems), validate_when_idle.after(CanvasSystems), poll_watcher),
        );
        if !app.world().contains_resource::<Editor>() {
            app.insert_resource(Editor::default());
        }
    }
}

/// Inserts [`EditorCanvas`] once the canvas entity exists. RMB drag pans (UE-like); MMB pan is
/// handled in `ui::input`, RMB click without a drag opens the palette.
fn attach_canvas(world: &mut World) {
    if world.contains_resource::<EditorCanvas>() {
        return;
    }
    let found = world.query_filtered::<Entity, With<ui::EditorCanvasMarker>>().iter(world).next();
    if let Some(canvas) = found {
        if let Some(mut interaction) = world.get_mut::<CanvasInteraction>(canvas) {
            interaction.pan_button = Some(PointerButton::Secondary);
        }
        world.insert_resource(EditorCanvas::new(canvas));
    }
}

/// Applies queued actions. Finished drags update the model without rebuilding the canvas.
pub fn apply_actions(mut editor: ResMut<Editor>, mut actions: ResMut<Actions>, ec: Option<ResMut<EditorCanvas>>) {
    if actions.0.is_empty() {
        return;
    }
    let mut ec = ec;
    for a in std::mem::take(&mut actions.0) {
        let moved = matches!(a, Action::Moved { .. });
        let before = editor.doc().map(|d| (d.id.clone(), d.revision));
        editor.apply(a);
        if moved
            && let Some(ec) = ec.as_mut()
            && ec.shown == before
        {
            ec.shown = editor.doc().map(|d| (d.id.clone(), d.revision));
        }
    }
}

fn validate_when_idle(mut editor: ResMut<Editor>, time: Res<Time>, mut quiet: Local<f32>) {
    if !editor.bypass_change_detection().needs_validation() {
        *quiet = 0.0;
        return;
    }
    *quiet += time.delta_secs();
    if *quiet >= VALIDATE_DELAY {
        editor.validate();
        *quiet = 0.0;
    }
}

fn poll_watcher(mut editor: ResMut<Editor>, time: Res<Time>, mut since: Local<f32>) {
    *since += time.delta_secs();
    if *since > 0.5 {
        *since = 0.0;
        editor.bypass_change_detection().poll_watcher();
    }
}

/// Errors of background openers, shown in the log.
struct OpenErrors(std::sync::mpsc::Sender<String>, std::sync::mpsc::Receiver<String>);

impl Default for OpenErrors {
    fn default() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        OpenErrors(tx, rx)
    }
}

fn open_externally(mut editor: ResMut<Editor>, errors: Local<OpenErrors>) {
    while let Ok(e) = errors.1.try_recv() {
        editor.error(e);
    }
    if editor.external.is_empty() {
        return;
    }
    for path in std::mem::take(&mut editor.external) {
        let tx = errors.0.clone();
        external::open(path, move |e| drop(tx.send(e)));
    }
}

fn copy_to_clipboard(mut editor: ResMut<Editor>, clipboard: Option<ResMut<bevy::clipboard::Clipboard>>) {
    let Some(text) = editor.copy_text.take() else { return };
    match clipboard.map(|mut c| c.set_text(text)) {
        Some(Ok(())) => editor.info("Copied to the clipboard"),
        Some(Err(e)) => editor.error(format!("cannot copy: {e}")),
        None => editor.error("cannot copy: no clipboard"),
    }
}

fn quit(mut editor: ResMut<Editor>, windows: Query<&Window>, mut exit: MessageWriter<AppExit>) {
    if !editor.quit {
        return;
    }
    if let Ok(w) = windows.single() {
        editor.config.window = (w.resolution.width() as u32, w.resolution.height() as u32);
    }
    editor.save_config();
    exit.write(AppExit::Success);
}

fn remember_window_size(mut resized: MessageReader<WindowResized>, mut editor: ResMut<Editor>) {
    if let Some(r) = resized.read().last() {
        editor.bypass_change_detection().config.window = (r.width as u32, r.height as u32);
    }
}

fn open_startup_project(mut editor: ResMut<Editor>, startup: Res<StartupProject>, mut actions: ResMut<Actions>) {
    let path = startup.0.clone().or_else(|| editor.config.recent.first().cloned());
    match path {
        Some(p) => {
            if let Err(e) = editor.open_project(&p) {
                editor.error(format!("cannot open {}: {e}", p.display()));
                actions.push(Action::OpenProjectDialog);
            }
        }
        None => actions.push(Action::OpenProjectDialog),
    }
    editor.bevy = shader_codegen_wesl::check::BevySources::discover();
    if editor.bevy.is_none() {
        editor.info("Bevy 0.20 WESL sources not found in the cargo registry: generated WESL is not compile-checked");
    }
}

/// Debug aid: `SHADER_EDITOR_SCREENSHOT=out.png` saves a screenshot after a few seconds and quits;
/// `SHADER_EDITOR_OPEN=env/rock_wet.material.ron` opens an asset at startup (`…ron#wesl` / `#rust`: its generated
/// file in a text tab, after the asset's graph).
/// `SHADER_EDITOR_DEMO=palette|wire|dialog|menu` additionally opens that popup before the shot.
fn debug_screenshot(
    mut frames: Local<u32>,
    mut editor: ResMut<Editor>,
    mut palette: ResMut<ui::palette::Palette>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok(path) = std::env::var("SHADER_EDITOR_SCREENSHOT") else { return };
    *frames += 1;
    if *frames == 2
        && let Ok(open) = std::env::var("SHADER_EDITOR_OPEN")
        && let (rel, lang) = open.split_once('#').map_or((open.as_str(), None), |(r, l)| (r, Some(l)))
        && let Some(id) = shader_project::assets::id_from_rel(rel)
    {
        editor.open_asset(&id);
        if let Some(lang) = lang.and_then(tab::Lang::from_label) {
            editor.show_generated(&id, lang);
        }
        if let Ok(sel) = std::env::var("SHADER_EDITOR_SELECT")
            && let Some(d) = editor.doc_mut()
            && let Some(n) = d.asset.nodes.iter().find(|(_, n)| n.kind == sel).map(|(k, _)| k.clone())
        {
            d.selection = [n].into();
            d.revision += 1;
            editor.ui.props += 1;
        }
    }
    if *frames == 150 {
        let output = editor
            .doc()
            .and_then(|d| d.asset.nodes.iter().find(|(k, _)| editor.is_output_node(k)).map(|(k, _)| k.clone()));
        match std::env::var("SHADER_EDITOR_DEMO").as_deref() {
            Ok("palette") => palette.show(Vec2::new(520.0, 300.0), Vec2::ZERO, None),
            Ok("wire") => {
                let link = output.map(|node| state::Link { node, pin: "roughness".into(), from_output: false });
                palette.show(Vec2::new(520.0, 300.0), Vec2::ZERO, link);
            }
            Ok("dialog") => actions.push(Action::NewAssetDialog(state::NewKind::Pbr3d)),
            Ok("menu") => ui::dialog::open_icon_menu(
                &mut commands,
                Vec2::new(120.0, 250.0),
                vec![
                    (Some(ui::icons::IconId::Pbr), "Open".into(), Action::None),
                    (Some(ui::icons::IconId::Edit), "Rename...".into(), Action::None),
                    (Some(ui::icons::IconId::Delete), "Delete...".into(), Action::None),
                    (None, "No icon".into(), Action::None),
                ],
            ),
            _ => {}
        }
    }
    if *frames == 240 {
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(path));
    }
    if *frames == 300 {
        exit.write(AppExit::Success);
    }
}

/// Runs the editor window; `project` is opened at startup.
pub fn run(project: Option<PathBuf>) {
    let config_path = UserConfig::default_path();
    let config = config_path.as_deref().map(UserConfig::load).unwrap_or_default();
    let (w, h) = config.window;
    let window = Window {
        title: "Shader Editor".into(),
        resolution: WindowResolution::new(w.max(800), h.max(500)),
        ..default()
    };
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin { primary_window: Some(window), close_when_requested: false, ..default() })
                .set(bevy::log::LogPlugin { filter: "wgpu=error,naga=warn,bevy_render=warn".into(), ..default() }),
        )
        .add_plugins((FeathersPlugins, fixes::FeathersFixesPlugin))
        .add_plugins((NoodlePlugins, NoodleDefaultStylePlugin))
        .insert_resource(UiTheme(create_dark_theme()))
        .insert_resource(ClearColor(Color::srgb_u8(24, 25, 29)))
        .insert_resource(Editor::with_config(config, config_path))
        .insert_resource(StartupProject(project))
        .add_plugins((EditorCorePlugin, ui::UiPlugin))
        .add_systems(Startup, open_startup_project)
        .add_systems(
            Update,
            (open_externally, copy_to_clipboard, quit, remember_window_size, debug_screenshot, demo::scripted_input),
        )
        .run();
}
