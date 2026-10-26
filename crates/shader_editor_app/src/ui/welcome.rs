//! Welcome screen, shown over the panels while no project is open (decision 0018): the logo, recent projects
//! (a click opens one), Open / New project and how to hook the generated code into the game.

use std::path::PathBuf;

use bevy::feathers::constants::fonts;
use bevy::prelude::*;
use bevy::text::FontSourceTemplate;
use bevy::ui_widgets::{Button, ScrollArea};

use super::icons::{IconId, icon};
use super::logo::logo;
use super::{HoverRow, OnActivate, TEXT, TEXT_DIM, VIEW_BG, button, text};
use crate::state::{Action, Editor};

#[derive(Component, Default, Clone)]
pub struct Welcome;

/// The list [`redraw_recent`] fills.
#[derive(Component, Default, Clone)]
pub struct RecentList;

pub struct WelcomePlugin;

impl Plugin for WelcomePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (show_welcome, redraw_recent));
    }
}

/// Fills the area below the main menu; hidden while a project is open.
pub fn welcome() -> impl Scene {
    let version = format!("Version {}", crate::EDITOR_VERSION);
    bsn! {
        Welcome
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            overflow: Overflow::scroll_y(),
            padding: UiRect::axes(px(16), px(32)),
        }
        ScrollArea
        BackgroundColor(VIEW_BG)
        ZIndex(10)
        Visibility::Hidden
        Children [
            @logo(256.0)
            --
            @text("Bevy Shader Editor", TEXT, 30.0) Node { margin: UiRect::top(px(8)) }
            --
            @text(version, TEXT_DIM, 13.0)
            --
            @text(crate::DISCLAIMER, TEXT_DIM, 11.0)
            Node { max_width: px(560), margin: UiRect::new(px(0), px(0), px(10), px(28)) }
            TextLayout { justify: Justify::Center }
            --
            Node { flex_direction: FlexDirection::Row, column_gap: px(40), align_items: AlignItems::Start }
            Children [
                Node { width: px(380), flex_direction: FlexDirection::Column, row_gap: px(6) }
                Children [
                    @text("Recent projects", TEXT, 16.0)
                    --
                    RecentList
                    Node { flex_direction: FlexDirection::Column, row_gap: px(2) }
                ]
                --
                Node { width: px(460), flex_direction: FlexDirection::Column, row_gap: px(8) }
                Children [
                    @text("Start", TEXT, 16.0)
                    --
                    Node { flex_direction: FlexDirection::Row, column_gap: px(8) }
                    Children [
                        @button("Open Project...", Action::OpenProjectDialog, false)
                        --
                        @button("New Project...", Action::NewProjectDialog, true)
                    ]
                    --
                    @setup_help()
                ]
            ]
        ]
    }
}

/// How to use a new project (README "Connecting to the game").
fn setup_help() -> impl Scene {
    let lines: Vec<Box<dyn Scene>> = vec![
        Box::new(text(
            "New Project sets up a Bevy game crate (the directory with its Cargo.toml): it creates assets/ and \
             shaders/ there. Graphs are saved in shaders/; every save regenerates WESL into assets/shaders/autogen/ \
             and the Rust crate crates/shaders/.",
            TEXT_DIM,
            13.0,
        )),
        Box::new(text("Then hook the generated crate into the game. In Cargo.toml:", TEXT_DIM, 13.0)),
        Box::new(code(
            "[workspace]\nmembers = [\"crates/shaders\"]\n\n[dependencies]\n\
             bevy = { version = \"0.20\", features = [\"file_watcher\"] }\nshaders = { path = \"crates/shaders\" }",
        )),
        Box::new(text("and add the plugin to the app (file_watcher enables hot reload):", TEXT_DIM, 13.0)),
        Box::new(code("App::new().add_plugins((DefaultPlugins, shaders::ShadersPlugin))")),
    ];
    bsn! {
        Node { flex_direction: FlexDirection::Column, row_gap: px(6), margin: UiRect::top(px(8)) }
        Children [ {lines} ]
    }
}

/// Help → Quick Start: the steps from an existing Bevy game to a material on screen (README "Hooking it up to a
/// game"), one tab per step: (title, optional).
pub const QUICK_START_STEPS: [(&str, bool); 7] = [
    ("Project", false),
    ("Material", false),
    ("Cargo.toml", false),
    ("Plugin", false),
    ("Spawn", false),
    ("Loading", true),
    ("Hot reload", false),
];

/// Body of the Quick Start dialog showing step `step` (clamped): the step tabs, then the step's text and code.
pub fn quick_start(step: usize) -> impl Scene {
    let step = step.min(QUICK_START_STEPS.len() - 1);
    let tabs: Vec<Box<dyn Scene>> = QUICK_START_STEPS
        .iter()
        .enumerate()
        .map(|(i, (title, optional))| -> Box<dyn Scene> { Box::new(step_tab(i, title, *optional, i == step)) })
        .collect();
    let (title, optional) = QUICK_START_STEPS[step];
    let mut heading = format!("Step {} of {}: {title}", step + 1, QUICK_START_STEPS.len());
    if optional {
        heading.push_str(" (optional)");
    }
    let mut lines: Vec<Box<dyn Scene>> = vec![Box::new(text(heading, TEXT, 15.0))];
    lines.extend(step_body(step));
    bsn! {
        Node { flex_direction: FlexDirection::Column, row_gap: px(12) }
        Children [
            Node { flex_direction: FlexDirection::Row, flex_wrap: FlexWrap::Wrap, column_gap: px(4), row_gap: px(4) }
            Children [ {tabs} ]
            --
            // A fixed minimum height: the dialog does not jump between short and long steps.
            Node { flex_direction: FlexDirection::Column, row_gap: px(8), min_height: px(440) }
            Children [ {lines} ]
        ]
    }
}

fn step_tab(i: usize, title: &str, optional: bool, selected: bool) -> impl Scene {
    let tip = format!("Step {}: {title}", i + 1);
    super::chip(format!("{}. {title}", i + 1), Action::QuickStart(i), selected, tip, optional.then_some("optional"))
}

fn step_body(step: usize) -> Vec<Box<dyn Scene>> {
    let p = |s: &'static str| -> Box<dyn Scene> { Box::new(text(s, TEXT, 13.0)) };
    let note = |s: &'static str| -> Box<dyn Scene> { Box::new(text(s, TEXT_DIM, 12.0)) };
    let c = |s: &'static str| -> Box<dyn Scene> { Box::new(code(s)) };
    match step {
        0 => vec![
            p("File > New Project... on the game crate (the directory with its Cargo.toml) creates assets/ and \
               shaders/ there; File > Open Project... opens one set up before. The editor owns these paths:"),
            c("my_game/\n\
               \x20 Cargo.toml                 # yours: the editor never edits it\n\
               \x20 shaders/                   # your graphs: *.material.ron, *.function.ron, *.texture.ron, ...\n\
               \x20 assets/shaders/autogen/    # generated WESL + baked.ron (do not edit)\n\
               \x20 assets/shaders/internal/   # built-in textures (generated)\n\
               \x20 crates/shaders/            # generated Rust crate (do not edit)"),
            note("Generated files carry an AUTOGENERATED marker; a file without it is never overwritten (E008)."),
        ],
        1 => vec![
            p("Create a material with Assets (or a right click in the Project tree), build the graph and save it \
               (Ctrl+S): every save regenerates WESL into assets/shaders/autogen/ and the Rust crate crates/shaders/."),
            p("Textures: drag an image from Game Assets onto the graph. It creates a Texture asset \
               (shaders/textures/<name>.texture.ron: path, sRGB, sampler) and a TextureSample node using it."),
            p("The same from a terminal or CI:"),
            c("shader-editor regen path/to/my_game             # what a save does, for the whole project\n\
               shader-editor check path/to/my_game --compile   # validate + compile WESL, writes nothing\n\
               shader-editor diff  path/to/my_game             # what regen would change"),
        ],
        2 => vec![
            p("Add the generated crate to the game's Cargo.toml (the editor never edits it):"),
            c("[workspace]\n\
               members = [\"crates/shaders\"]\n\n\
               [dependencies]\n\
               bevy = { version = \"0.20\", features = [\"file_watcher\"] }   # file_watcher: hot reload\n\
               shaders = { path = \"crates/shaders\" }"),
            note(
                "If Bevy is declared in [workspace.dependencies], crates/shaders takes it with \
                  bevy = { workspace = true }, so the game's Bevy version and features apply.",
            ),
        ],
        3 => vec![
            p("Add ShadersPlugin after DefaultPlugins: it registers every generated material type, reserves a handle \
               per material (BakedMaterials) and loads the baked values and textures."),
            c("use bevy::prelude::*;\n\n\
               fn main() {\n\
               \x20   App::new()\n\
               \x20       .add_plugins((DefaultPlugins, shaders::ShadersPlugin))\n\
               \x20       .add_systems(Startup, spawn)\n\
               \x20       .run();\n\
               }"),
        ],
        4 => vec![
            p("Every material is a field of the BakedMaterials resource, by its path under shaders/ \
               (shaders/env/rock_wet.material.ron is baked.env.rock_wet). The handles exist from the start; the \
               materials appear behind them on their own, with the values set in the graph."),
            c("use shaders::BakedMaterials;\n\n\
               fn spawn(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, baked: Res<BakedMaterials>) {\n\
               \x20   // Pbr3d needs tangents for a Normal input.\n\
               \x20   let sphere = Sphere::new(1.0).mesh().uv(48, 24).with_generated_tangents().unwrap();\n\
               \x20   commands.spawn((Mesh3d(meshes.add(sphere)), MeshMaterial3d(baked.env.rock_wet.clone())));\n\
               \x20   commands.spawn((Camera3d::default(), Transform::from_xyz(0.0, 1.0, 4.0)));\n\
               }\n\n\
               // Unlit3d: MeshMaterial3d(baked.fx.hologram.clone()) as well.\n\
               // Sprite2d: (Sprite::from_image(image), SpriteMaterial(baked.ui.dissolve.clone())).\n\
               // By id, e.g. from your own asset data (None for an unknown id or another type):\n\
               // baked.get::<RockWetMaterial>(\"env/rock_wet\"), tex.get(\"textures/rock_albedo\")."),
            note(
                "To change a value, edit the graph: the game picks it up (step 7). The material constructors are \
                  internal to the generated crate.",
            ),
        ],
        5 => vec![
            p("Loading starts on its own in the first frame. Call load() in Startup yourself to use BakedTextures \
               there (every Texture asset by path: tex.textures.sprite) or to show a loading screen:"),
            c("use shaders::{BakedMaterialsLoader, BakedMaterialsProgress, BakedMaterialsReady, BakedTextures};\n\n\
               // .add_systems(Startup, (load, spawn).chain()).add_systems(Update, loading_screen)\n\
               fn load(mut loader: BakedMaterialsLoader) {\n\
               \x20   let textures = loader.load(); // idempotent; fills BakedTextures at once\n\
               \x20   info!(\"loading {textures} textures\");\n\
               }\n\n\
               fn spawn(mut commands: Commands, tex: Res<BakedTextures>) {\n\
               \x20   commands.spawn(Sprite::from_image(tex.textures.sprite.clone()));\n\
               }\n\n\
               fn loading_screen(\n\
               \x20   mut progress: MessageReader<BakedMaterialsProgress>,\n\
               \x20   mut ready: MessageReader<BakedMaterialsReady>,\n\
               ) {\n\
               \x20   for p in progress.read() {\n\
               \x20       info!(\"{} / {} textures ({} failed)\", p.loaded + p.failed, p.total, p.failed);\n\
               \x20   }\n\
               \x20   if ready.read().next().is_some() {\n\
               \x20       info!(\"all textures loaded\");\n\
               \x20   }\n\
               }"),
            note(
                "Materials render before that: Bevy shows each one once its images are in. A texture that fails to \
                  load is replaced by the built-in default texture.",
            ),
        ],
        _ => vec![
            p("Run the game with file_watcher and keep the editor open: every save regenerates the files and the \
               game reloads them without a restart."),
            c("cd my_game && cargo run"),
            p("Reloaded live: the shader code, parameter values, the texture of a parameter, a Texture asset pointed \
               at another file, sampler settings."),
            p("Need a rebuild (cargo run again): a new / removed / renamed material or parameter, a changed \
               material kind or blend settings, sRGB / Expand grayscale of a Texture asset, new BakedTextures fields."),
            note("Out-of-sync values keep the compiled ones and log \"rebuild the game\" once."),
        ],
    }
}

fn code(s: &'static str) -> impl Scene {
    bsn! {
        Node { padding: UiRect::axes(px(10), px(6)), border_radius: BorderRadius::all(px(4)) }
        BackgroundColor(Color::srgb_u8(36, 38, 44))
        Children [
            Text(s)
            TextFont { font: FontSourceTemplate::Handle(fonts::MONO), font_size: FontSize::Px(12.0) }
            TextColor(TEXT)
        ]
    }
}

fn recent_row(path: PathBuf) -> impl Scene {
    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    let (full, tip) = (path.display().to_string(), super::tooltip::Tooltip(format!("Open {}", path.display())));
    let (act, hover) = (OnActivate(Action::OpenProject(path)), HoverRow { base: Color::NONE });
    bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(10),
            padding: UiRect::axes(px(8), px(5)),
            border_radius: BorderRadius::all(px(4)),
        }
        BackgroundColor(Color::NONE)
        Button
        act
        hover
        tip
        Children [
            @icon(IconId::Folder, 16.0, TEXT)
            --
            Node { flex_direction: FlexDirection::Column, min_width: px(0) }
            Children [ @text(name, TEXT, 14.0) -- @text(full, TEXT_DIM, 12.0) ]
        ]
    }
}

fn show_welcome(editor: Res<Editor>, mut q: Query<&mut Visibility, With<Welcome>>) {
    let want = if editor.project.is_none() { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut q {
        v.set_if_neq(want);
    }
}

fn redraw_recent(
    editor: Res<Editor>,
    list: Query<Entity, With<RecentList>>,
    mut last: Local<Option<Vec<PathBuf>>>,
    mut commands: Commands,
) {
    let recent = &editor.config.recent_projects;
    if last.as_ref() == Some(recent) {
        return;
    }
    let Ok(list) = list.single() else { return };
    *last = Some(recent.clone());
    commands.entity(list).despawn_children();
    if recent.is_empty() {
        commands.spawn_scene(bsn! { @text("No recent projects yet.", TEXT_DIM, 13.0) ChildOf(list) });
    }
    for p in recent.clone() {
        commands.spawn_scene(bsn! { @recent_row(p) ChildOf(list) });
    }
}
