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
pub mod consts_doc;
pub mod curve_doc;
mod demo;
pub mod doc;
pub mod doc_kind;
pub mod droppable;
pub mod editors;
mod external;
mod fixes;
pub mod game_image;
pub mod gradients_doc;
pub mod highlight;
pub mod history;
pub mod inline;
pub mod keys;
pub mod search;
pub mod state;
pub mod tab;
pub mod texture_doc;
pub mod ui;

use canvas::{CanvasPlugin, CanvasSystems, EditorCanvas};
use config::UserConfig;
use state::{Action, Actions, Editor};

/// Editor version (Help → About, `--version`); CI sets it on release.
pub const EDITOR_VERSION: &str = "0.2";
pub const PROJECT_URL: &str = "https://github.com/fe3dback/bevy-shader-vibe";
pub const AUTHORS: &str = "claude code / fe3dback";
pub const LICENSE: &str = "MIT";
/// Shown small and dimmed on the welcome screen and in About.
pub const DISCLAIMER: &str = "Unofficial, community-driven editor, not supported by the Bevy developers. It will most \
     likely be discontinued once the official Bevy editor ships with a built-in material editor.";

/// Seconds of quiet before live validation runs.
const VALIDATE_DELAY: f32 = 0.25;

/// Project to open at startup (CLI argument, else the most recent one); without one the welcome screen shows.
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
            // UE: dragging from a connected input starts a new wire; Ctrl+drag lifts the pin's wires to move them
            // (decision 0032, design §5.3), "Ctrl" being Ctrl or Super as in the hotkeys.
            interaction.detach_wires = false;
            interaction.move_wires_keys =
                vec![KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::SuperLeft, KeyCode::SuperRight];
            // Dragged nodes snap to the grid (design §4.1); comment frames carry their nodes (`canvas::frame_scene`).
            interaction.snap = Some(doc::GRID_SNAP);
            // A dragged or lifted wire takes the nearest pin it may end on within 40 screen px (design §6.3).
            interaction.wire_target_radius = Some(canvas::WIRE_TARGET_RADIUS);
        }
        // A hovered pin's dot grows and lightens (design §4.8), also under a dragged wire.
        let hover = PortHoverStyle { scale: canvas::PIN_HOVER_SCALE, lighten: canvas::PIN_HOVER_LIGHTEN };
        world.entity_mut(canvas).insert(hover);
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

/// Runs the native folder picker the Open / New project dialogs ask for (`Editor::pick_folder`) on its own thread;
/// a choice submits the dialog, a cancel leaves the dialog's text field for typing / pasting the path.
fn pick_folder(
    mut editor: ResMut<Editor>,
    mut actions: ResMut<Actions>,
    mut pending: Local<Option<std::sync::mpsc::Receiver<Option<PathBuf>>>>,
) {
    if let Some(rx) = pending.as_ref() {
        match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // One picker at a time.
                if editor.pick_folder {
                    editor.pick_folder = false;
                }
                return;
            }
            Ok(Some(path)) => actions.push(Action::FolderPicked(path)),
            Ok(None) | Err(_) => {}
        }
        *pending = None;
    }
    if !editor.pick_folder {
        return;
    }
    editor.pick_folder = false;
    // Screenshot runs are unattended: no system dialog on the desktop.
    if std::env::var_os("SHADER_EDITOR_SCREENSHOT").is_some() {
        return;
    }
    let start = editor.project.as_ref().map(|p| p.root.clone()).or_else(std::env::home_dir);
    let (tx, rx) = std::sync::mpsc::channel();
    *pending = Some(rx);
    std::thread::spawn(move || {
        let mut dialog = rfd::AsyncFileDialog::new().set_title("Select the game directory (with Cargo.toml)");
        if let Some(start) = start {
            dialog = dialog.set_directory(start);
        }
        let picked = bevy::tasks::block_on(dialog.pick_folder());
        drop(tx.send(picked.map(|h| h.path().to_path_buf())));
    });
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
    let path = startup.0.clone().or_else(|| editor.config.recent_projects.first().cloned());
    if let Some(p) = path {
        actions.push(Action::OpenProject(p));
    }
    editor.bevy = shader_codegen_wesl::check::BevySources::discover();
    if editor.bevy.is_none() {
        editor.info("Bevy 0.20 WESL sources not found in the cargo registry: generated WESL is not compile-checked");
    }
}

/// Debug aid: `SHADER_EDITOR_SCREENSHOT=out.png` saves a screenshot after a few seconds and quits;
/// `SHADER_EDITOR_OPEN=env/rock_wet.material.ron` opens an asset at startup (`…ron#wesl` / `#rust`: its generated
/// file in a text tab, after the asset's graph; `@game/textures/rock_albedo.ktx2`: a game image's tab;
/// `@internal/default`: a built-in texture's read-only Texture tab); `SHADER_EDITOR_SELECT=stdlib::Lerp` selects the
/// first node of that kind, or the comment frame of that id (`c01`); on a gradient collection the slot at a position,
/// `0:s1` also its stop 1, `0:m1` its midpoint 1.
/// `SHADER_EDITOR_DEMO=palette|wire|dialog|about|quick_start[:N]|texture_channel:N|open_project|welcome|menu[:name]|tab_menu|game_file_menu[:2]|gradient_color`
/// additionally opens that popup before the shot (`menu`: the main menu's File, `menu:view`: its View, …;
/// `game_file_menu`: the RMB menu of the first game image with a Texture asset — its item disabled —, `:2` of the
/// first without one; `welcome`: closes the project; `gradient_color`: selects the first stop of a gradients tab's
/// selected slot and opens its color picker).
fn debug_screenshot(
    mut frames: Local<u32>,
    mut editor: ResMut<Editor>,
    mut palette: ResMut<ui::palette::Palette>,
    mut actions: ResMut<Actions>,
    menus: Query<(Entity, &ui::MainMenu)>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok(path) = std::env::var("SHADER_EDITOR_SCREENSHOT") else { return };
    *frames += 1;
    if *frames == 2
        && let Ok(open) = std::env::var("SHADER_EDITOR_OPEN")
        && let Some(path) = open.strip_prefix(tab::GAME_KEY_PREFIX)
    {
        editor.open_game_image(path);
    } else if *frames == 2
        && let Ok(open) = std::env::var("SHADER_EDITOR_OPEN")
        && open.starts_with(shader_graph_core::texture::INTERNAL_PREFIX)
    {
        editor.open_asset(&shader_project::AssetId::new(shader_project::AssetType::Texture, open));
    } else if *frames == 2
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
        {
            // A node kind, or the id of a comment frame.
            if let Some(n) = d.asset.nodes.iter().find(|(_, n)| n.kind == sel).map(|(k, _)| k.clone()) {
                d.set_selection([n]);
            } else if d.asset.comments.iter().any(|f| f.id == sel) {
                d.selected_frame = Some(sel);
            }
            d.revision += 1;
            editor.ui.props += 1;
        } else if let Ok(sel) = std::env::var("SHADER_EDITOR_SELECT")
            && editor.gradients_doc().is_some()
        {
            // `<slot>`, `<slot>:s<stop>` or `<slot>:m<segment>`.
            let (slot, item) = sel.split_once(':').map_or((sel.as_str(), None), |(s, i)| (s, Some(i)));
            if let Ok(slot) = slot.parse() {
                editor.apply(Action::Gradients(state::GradientsAction::SelectSlot(slot)));
            }
            let pick = |prefix| item.and_then(|i| i.strip_prefix(prefix)).and_then(|i| i.parse().ok());
            let sel = match (pick('s'), pick('m')) {
                (Some(i), _) => Some(gradients_doc::GradientSel::Stop(i)),
                (_, Some(i)) => Some(gradients_doc::GradientSel::Mid(i)),
                _ => None,
            };
            editor.apply(Action::Gradients(state::GradientsAction::Select(sel)));
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
            Ok("about") => actions.push(Action::About),
            Ok("quick_start") => actions.push(Action::QuickStart(0)),
            Ok(d) if d.starts_with("texture_channel:") => {
                actions.push(Action::PreviewChannel(d["texture_channel:".len()..].parse::<usize>().ok()))
            }
            Ok(d) if d.starts_with("quick_start:") => actions
                .push(Action::QuickStart(d["quick_start:".len()..].parse::<usize>().unwrap_or(1).saturating_sub(1))),
            Ok("open_project") => actions.push(Action::OpenProjectDialog),
            Ok("welcome") => actions.push(Action::CloseProject(state::Unsaved::Discard)),
            Ok(d) if d == "menu" || d.starts_with("menu:") => {
                let name = d.strip_prefix("menu:").unwrap_or("file");
                if let Some((menu, _)) = menus.iter().find(|(_, m)| m.label().eq_ignore_ascii_case(name)) {
                    let open = bevy::ui_widgets::MenuAction::Open(bevy::input_focus::tab_navigation::NavAction::First);
                    commands.trigger(bevy::ui_widgets::MenuEvent { source: menu, action: open });
                }
            }
            Ok("tab_menu") => ui::dialog::open_icon_menu(&mut commands, Vec2::new(260.0, 60.0), ui::tabs::tab_menu(0)),
            Ok("gradient_color") => {
                // The first stop of the selected slot (`SHADER_EDITOR_SELECT=<slot>` picks one with stops).
                let first = Some(gradients_doc::GradientSel::Stop(0));
                editor.apply(Action::Gradients(state::GradientsAction::Select(first)));
                ui::gradient_view::open_stop_color(&mut commands, 0);
            }
            Ok(d) if d == "game_file_menu" || d == "game_file_menu:2" => {
                // The RMB menu of the first game image with a Texture asset (disabled item) or, `:2`, without one.
                let has = |f: &&String| shader_project::game_assets::texture_for_file(&editor.disk, f).is_some();
                let file = if d.ends_with(":2") {
                    editor.game_files.iter().find(|f| !has(f))
                } else {
                    editor.game_files.iter().find(has)
                };
                if let Some(f) = file {
                    let row = ui::tree::TreeRow {
                        tree: ui::tree::TreeId::GameAssets,
                        item: ui::tree::TreeItem::GameFile(f.clone()),
                    };
                    ui::dialog::open_menu_entries(
                        &mut commands,
                        Vec2::new(240.0, 300.0),
                        ui::tree::row_menu(&editor, &row),
                    );
                }
            }
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
    if let Some(Err(e)) = config_path.as_deref().map(UserConfig::ensure_dir) {
        eprintln!("warning: cannot create the config directory: {e}");
    }
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
            (
                open_externally,
                pick_folder,
                copy_to_clipboard,
                quit,
                remember_window_size,
                debug_screenshot,
                demo::scripted_input,
            ),
        )
        .run();
}
