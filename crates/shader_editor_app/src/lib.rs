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
pub mod doc;
mod fixes;
pub mod search;
pub mod state;
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

/// Inserts [`EditorCanvas`] once the canvas entity exists.
fn attach_canvas(world: &mut World) {
    if world.contains_resource::<EditorCanvas>() {
        return;
    }
    let found = world.query_filtered::<Entity, With<ui::EditorCanvasMarker>>().iter(world).next();
    if let Some(canvas) = found {
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

fn open_externally(mut editor: ResMut<Editor>) {
    if editor.external.is_empty() {
        return;
    }
    for path in std::mem::take(&mut editor.external) {
        let result = if cfg!(target_os = "macos") {
            std::process::Command::new("open").arg(&path).spawn()
        } else if cfg!(target_os = "windows") {
            std::process::Command::new("cmd").args(["/C", "start", ""]).arg(&path).spawn()
        } else {
            std::process::Command::new("xdg-open").arg(&path).spawn()
        };
        if let Err(e) = result {
            editor.error(format!("cannot open {}: {e}", path.display()));
        }
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
/// `SHADER_EDITOR_OPEN=env/rock_wet.material.ron` opens an asset at startup.
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
        && let Ok(rel) = std::env::var("SHADER_EDITOR_OPEN")
        && let Some(id) = shader_project::assets::id_from_rel(&rel)
    {
        editor.open_asset(&id);
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
            Ok("menu") => ui::dialog::open_menu(
                &mut commands,
                Vec2::new(120.0, 250.0),
                vec![
                    ("Open".into(), Action::None),
                    ("Rename...".into(), Action::None),
                    ("Delete...".into(), Action::None),
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
        .add_systems(Update, (open_externally, quit, remember_window_size, debug_screenshot))
        .run();
}
