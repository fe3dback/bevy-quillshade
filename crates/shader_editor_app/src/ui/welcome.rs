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
