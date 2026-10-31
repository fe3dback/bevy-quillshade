//! The editor app (design §6.1): [`EditorCorePlugin`] (logic without a window) and [`run`] (the window), with the
//! domain-neutral systems around them and the core half of the debug screenshot hooks.

use std::path::PathBuf;
use std::sync::Arc;

use crate::action::{Action, Actions, Unsaved};
use crate::config::UserConfig;
use crate::menu::MainMenu;
use crate::{external, fixes};
use bevy::app::PluginGroupBuilder;
use bevy::feathers::FeathersPlugins;
use bevy::feathers::dark_theme::create_dark_theme;
use bevy::feathers::theme::UiTheme;
use bevy::prelude::*;
use bevy::window::{WindowResized, WindowResolution};

use crate::domain::{DomainList, EditorDomain};
use crate::state::editor::Editor;
use crate::ui::shell::UiPlugin;

/// Project to open at startup (CLI argument, else the most recent one); without one the welcome screen shows.
#[derive(Resource, Default)]
struct StartupProject(Option<PathBuf>);

/// Window size forced from the command line (`--width` / `--height`): the saved size in the user config is kept.
#[derive(Resource)]
struct FixedWindowSize;

/// Editor logic without windowing: state, actions, the domains' logic systems (`DomainRegistrar::logic`). Used by
/// the app and UI tests. Needs the [`Editor`] resource (`Editor::new` with the domain list): it puts the editor's
/// [`DomainList`] and the registered editor kinds ([`crate::editors::EditorKinds`], which the window areas and
/// the input contexts are built from, design §6.2) in the app.
pub struct EditorCorePlugin;

impl Plugin for EditorCorePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Actions>()
            .add_systems(Update, (poll_watcher, apply_actions, crate::preview::sync_previews).chain());
        let Some(editor) = app.world().get_resource::<Editor>() else {
            panic!("EditorCorePlugin needs the Editor resource (Editor::new with the domain list) inserted first");
        };
        let list = editor.domain_list.clone();
        app.insert_resource(crate::preview::PreviewTracker::new(list.previews()));
        app.insert_resource(list.kind_defs());
        app.insert_resource(list.as_ref().clone());
        for add in list.domains().iter().flat_map(|d| d.logic.clone()) {
            add(app);
        }
    }
}

/// The editor's plugins without a window (what headless UI tests build on): [`EditorCorePlugin`], which adds the
/// domains' logic. The window app adds [`UiPlugin`] (the shell and the domains' window parts) on top ([`run`]).
pub struct EditorPlugins;

impl PluginGroup for EditorPlugins {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::start::<Self>().add(EditorCorePlugin)
    }
}

/// Applies queued actions.
pub fn apply_actions(mut editor: ResMut<Editor>, mut actions: ResMut<Actions>) {
    if actions.0.is_empty() {
        return;
    }
    for a in std::mem::take(&mut actions.0) {
        editor.apply(a);
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

fn quit(
    mut editor: ResMut<Editor>,
    windows: Query<&Window>,
    fixed: Option<Res<FixedWindowSize>>,
    mut exit: MessageWriter<AppExit>,
) {
    if !editor.quit {
        return;
    }
    if let (None, Ok(w)) = (fixed, windows.single()) {
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

fn open_startup_project(editor: Res<Editor>, startup: Res<StartupProject>, mut actions: ResMut<Actions>) {
    let path = startup.0.clone().or_else(|| editor.config.recent_projects.first().cloned());
    if let Some(p) = path {
        actions.push(Action::OpenProject(p));
    }
}

/// Debug aid, core half: `SHADER_EDITOR_SCREENSHOT=out.png` saves a screenshot after a few seconds and quits;
/// `SHADER_EDITOR_DEMO=about|quick_start[:N]|open_project|welcome|menu[:name]|tab_menu|settings_dirty` additionally
/// opens that popup before the shot (`menu`: the main menu's File, `menu:view`: its View, …; `welcome`: closes the
/// project; `settings_dirty`: edits the active Project Settings tab without saving). The
/// domains add their own `SHADER_EDITOR_OPEN` / `SHADER_EDITOR_SELECT` / `SHADER_EDITOR_DEMO` values on the same frames
/// (the asset opens on frame 2, popups on frame 150). `SHADER_EDITOR_DOMAIN=<id>` switches to that domain on frame 2,
/// after the domains' `SHADER_EDITOR_OPEN` (this system runs after [`apply_actions`], so the switch applies next
/// frame).
fn debug_screenshot(
    mut frames: Local<u32>,
    mut actions: ResMut<Actions>,
    list: Res<DomainList>,
    menus: Query<(Entity, &MainMenu)>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok(path) = std::env::var("SHADER_EDITOR_SCREENSHOT") else { return };
    *frames += 1;
    if *frames == 2
        && let Ok(domain) = std::env::var("SHADER_EDITOR_DOMAIN")
    {
        match list.ids().find(|id| id.as_str() == domain) {
            Some(id) => actions.push(Action::SwitchDomain(id)),
            None => error!("SHADER_EDITOR_DOMAIN: no domain {domain:?}"),
        }
    }
    if *frames == 150 {
        match std::env::var("SHADER_EDITOR_DEMO").as_deref() {
            Ok("about") => actions.push(Action::About),
            Ok("quick_start") => actions.push(Action::QuickStart(0)),
            Ok(d) if d.starts_with("quick_start:") => actions
                .push(Action::QuickStart(d["quick_start:".len()..].parse::<usize>().unwrap_or(1).saturating_sub(1))),
            Ok("open_project") => actions.push(Action::OpenProjectDialog),
            Ok("welcome") => actions.push(Action::CloseProject(Unsaved::Discard)),
            Ok(d) if d == "menu" || d.starts_with("menu:") => {
                let name = d.strip_prefix("menu:").unwrap_or("file");
                if let Some((menu, _)) = menus.iter().find(|(_, m)| m.label().eq_ignore_ascii_case(name)) {
                    let open = bevy::ui_widgets::MenuAction::Open(bevy::input_focus::tab_navigation::NavAction::First);
                    commands.trigger(bevy::ui_widgets::MenuEvent { source: menu, action: open });
                }
            }
            // The active Project Settings tab edited, not saved: header date on, the first domain not used in game.
            Ok("settings_dirty") => {
                use crate::dynval::PropValue::Bool;
                use crate::settings::SettingsProp::{Enabled, HeaderDate};
                use crate::ui::props::set_action;
                actions.push(set_action(&HeaderDate.into(), Bool(true), false));
                if let Some(first) = list.real_ids().next() {
                    actions.push(set_action(&Enabled(first).into(), Bool(false), false));
                }
            }
            Ok("tab_menu") => {
                crate::ui::dialog::open_icon_menu(&mut commands, Vec2::new(260.0, 60.0), crate::ui::tabs::tab_menu(0))
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

/// Runs the editor window with the registered `domains` (`src/main.rs`'s list, design §6.1); `project` is opened at
/// startup. `window_size` (logical px) overrides the saved window size for this run without replacing it in the user
/// config.
pub fn run(domains: Vec<Box<dyn EditorDomain>>, project: Option<PathBuf>, window_size: Option<(u32, u32)>) {
    let domains = Arc::new(DomainList::new(domains));
    let config_path = UserConfig::default_path();
    if let Some(Err(e)) = config_path.as_deref().map(UserConfig::ensure_dir) {
        eprintln!("warning: cannot create the config directory: {e}");
    }
    let config = config_path.as_deref().map(UserConfig::load).unwrap_or_default();
    let (w, h) = window_size.unwrap_or((config.window.0.max(800), config.window.1.max(500)));
    let window = Window { title: "Shader Editor".into(), resolution: WindowResolution::new(w, h), ..default() };
    let mut app = App::new();
    if window_size.is_some() {
        app.insert_resource(FixedWindowSize);
    }
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: Some(window), close_when_requested: false, ..default() })
            .set(bevy::log::LogPlugin { filter: "wgpu=error,naga=warn,bevy_render=warn".into(), ..default() }),
    )
    .add_plugins((FeathersPlugins, fixes::FeathersFixesPlugin))
    .insert_resource(UiTheme(create_dark_theme()))
    .insert_resource(ClearColor(Color::srgb_u8(24, 25, 29)))
    .insert_resource(Editor::new(domains).with_config(config, config_path))
    .insert_resource(StartupProject(project))
    .add_plugins((EditorPlugins, UiPlugin))
    .add_systems(Startup, open_startup_project)
    .add_systems(
        Update,
        (
            open_externally,
            pick_folder,
            copy_to_clipboard,
            quit,
            remember_window_size.run_if(not(resource_exists::<FixedWindowSize>)),
        ),
    );
    if std::env::var_os("SHADER_EDITOR_SCREENSHOT").is_some() || std::env::var_os("SHADER_EDITOR_INPUT").is_some() {
        app.add_systems(Update, (debug_screenshot.after(apply_actions), crate::demo::scripted_input));
    }
    app.run();
}
