//! Window layout (spec §9): toolbar, asset browser (left), tabs + canvas + log (center), properties (right).

use bevy::feathers::constants::size;
use bevy::feathers::controls::{ButtonVariant, FeathersButton, FeathersToolButton};
use bevy::feathers::display::caption;
use bevy::feathers::theme::ThemeBackgroundColor;
use bevy::feathers::tokens;
use bevy::input_focus::tab_navigation::TabGroup;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, ScrollArea};
use bevy_noodle::style::kit;

use crate::state::{Action, Actions, Editor, NewKind, ViewRequest};

pub mod browser;
pub mod dialog;
pub mod input;
pub mod log;
pub mod palette;
pub mod props;
pub mod tabs;

/// Pushes its action when the widget is activated (buttons, menu items).
#[derive(Component, Default, Clone)]
pub struct OnActivate(pub Action);

#[derive(Component, Default, Clone)]
pub struct CanvasHost;

#[derive(Component, Default, Clone)]
pub struct EditorCanvasMarker;

#[derive(Component, Default, Clone)]
pub struct EmptyHint;

#[derive(Component, Default, Clone)]
pub struct StatusText;

pub const TEXT: Color = Color::srgb(0.86, 0.87, 0.90);
pub const TEXT_DIM: Color = Color::srgb(0.55, 0.57, 0.62);
pub const ROW_HOVER: Color = Color::srgba(1.0, 1.0, 1.0, 0.06);
pub const ROW_ACTIVE: Color = Color::srgba(0.35, 0.55, 0.95, 0.30);
pub const PANEL_BG: Color = Color::srgb_u8(32, 34, 39);
pub const HEADER_BG: Color = Color::srgb_u8(40, 42, 48);
pub const BORDER: Color = Color::srgb_u8(20, 21, 24);

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            browser::BrowserPlugin,
            tabs::TabsPlugin,
            props::PropsPlugin,
            log::LogPlugin,
            dialog::DialogPlugin,
            palette::PalettePlugin,
            input::InputPlugin,
        ))
        .add_systems(Startup, spawn_layout)
        .add_systems(Update, (show_empty_hint, show_status))
        .add_observer(activate_action)
        .add_observer(row_hover_on)
        .add_observer(row_hover_off);
    }
}

fn activate_action(ev: On<Activate>, q: Query<&OnActivate>, mut actions: ResMut<Actions>) {
    if let Ok(a) = q.get(ev.event_target()) {
        actions.push(a.0.clone());
    }
}

/// Rows that highlight under the pointer.
#[derive(Component, Default, Clone)]
pub struct HoverRow {
    pub base: Color,
}

fn row_hover_on(ev: On<PointerOver>, mut q: Query<(&HoverRow, &mut BackgroundColor)>) {
    if let Ok((r, mut bg)) = q.get_mut(ev.event_target())
        && r.base.alpha() < 0.2
    {
        bg.0 = ROW_HOVER;
    }
}

fn row_hover_off(ev: On<PointerOut>, mut q: Query<(&HoverRow, &mut BackgroundColor)>) {
    if let Ok((r, mut bg)) = q.get_mut(ev.event_target()) {
        bg.0 = r.base;
    }
}

/// Small text button that pushes `action`.
pub fn tool_button(label: impl Into<String>, action: Action) -> impl Scene {
    let label = label.into();
    let act = OnActivate(action);
    bsn! {
        @FeathersToolButton { @variant: ButtonVariant::Plain, @caption: bsn! { @caption(label) } }
        act
    }
}

pub fn button(label: impl Into<String>, action: Action, primary: bool) -> impl Scene {
    let label = label.into();
    let act = OnActivate(action);
    let variant = if primary { ButtonVariant::Primary } else { ButtonVariant::Normal };
    bsn! {
        @FeathersButton { @variant: variant, @caption: bsn! { @caption(label) } }
        act
    }
}

/// Plain text with an explicit color and size.
pub fn text(s: impl Into<String>, color: Color, font_size: f32) -> impl Scene {
    let s = s.into();
    bsn! {
        Text(s)
        TextFont { font_size: FontSize::Px(font_size) }
        TextColor(color)
        Pickable { should_block_lower: false, is_hoverable: false }
    }
}

fn header(title: &'static str) -> impl Scene {
    bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(4),
            padding: UiRect::axes(px(8), px(4)),
            min_height: size::HEADER_HEIGHT,
        }
        BackgroundColor(HEADER_BG)
        Children [ @text(title, TEXT, 14.0) ]
    }
}

fn toolbar() -> impl Scene {
    let buttons: Vec<Box<dyn Scene>> = vec![
        Box::new(tool_button("Open...", Action::OpenProjectDialog)),
        Box::new(tool_button("Save", Action::Save)),
        Box::new(tool_button("Save all", Action::SaveAll)),
        Box::new(tool_button("Regenerate", Action::RegenAll)),
        Box::new(sep()),
        Box::new(tool_button("Undo", Action::Undo)),
        Box::new(tool_button("Redo", Action::Redo)),
        Box::new(sep()),
        Box::new(tool_button("Auto-layout", Action::AutoLayout)),
        Box::new(tool_button("Comment", Action::AddComment(None))),
        Box::new(tool_button("Fit", Action::View(ViewRequest::FitAll))),
    ];
    bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(2),
            padding: UiRect::axes(px(6), px(3)),
            border: UiRect::bottom(px(1)),
        }
        BackgroundColor(HEADER_BG)
        BorderColor::all(BORDER)
        Children [
            {buttons}
            --
            Node { flex_grow: 1.0 }
            --
            StatusText
            Text("")
            TextFont { font_size: FontSize::Px(12.0) }
            TextColor(TEXT_DIM)
            Node { margin: UiRect::right(px(6)) }
        ]
    }
}

fn sep() -> impl Scene {
    bsn! { Node { width: px(1), height: px(18), margin: UiRect::horizontal(px(4)) } BackgroundColor(BORDER) }
}

/// Scrollable column used by the panels; `marker` tags the entity the panel fills.
pub fn scroll_column() -> impl Scene {
    bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            min_height: px(0),
            overflow: Overflow::scroll_y(),
            padding: UiRect::all(px(4)),
            row_gap: px(1),
        }
        ScrollArea
    }
}

fn spawn_layout(mut commands: Commands) {
    commands.spawn(Camera2d);
    let new_buttons: Vec<Box<dyn Scene>> = NewKind::ALL
        .iter()
        .map(|k| -> Box<dyn Scene> {
            let short = match k {
                NewKind::Pbr3d => "+Pbr",
                NewKind::Unlit3d => "+Unlit",
                NewKind::Sprite2d => "+2D",
                NewKind::Function => "+Fn",
            };
            Box::new(tool_button(short, Action::NewAssetDialog(*k)))
        })
        .collect();
    commands.spawn_scene(bsn! {
        Node { width: percent(100), height: percent(100), flex_direction: FlexDirection::Column }
        ThemeBackgroundColor(tokens::WINDOW_BG)
        TabGroup
        Children [
            @toolbar()
            --
            Node { flex_grow: 1.0, flex_direction: FlexDirection::Row, min_height: px(0) }
            Children [
                // Asset browser.
                Node { width: px(270), flex_direction: FlexDirection::Column, border: UiRect::right(px(1)) }
                BackgroundColor(PANEL_BG)
                BorderColor::all(BORDER)
                Children [
                    @header("Assets")
                    --
                    Node { flex_direction: FlexDirection::Row, flex_wrap: FlexWrap::Wrap, padding: UiRect::all(px(3)), column_gap: px(2) }
                    Children [ {new_buttons} -- @tool_button("+Folder", Action::NewFolderDialog) ]
                    --
                    @scroll_column() browser::BrowserBody
                ]
                --
                // Center: tabs, canvas, log.
                Node { flex_grow: 1.0, flex_direction: FlexDirection::Column, min_width: px(0) }
                Children [
                    tabs::TabsBar
                    Node { flex_direction: FlexDirection::Row, min_height: px(30), align_items: AlignItems::End, column_gap: px(1), padding: UiRect::left(px(4)) }
                    BackgroundColor(HEADER_BG)
                    --
                    CanvasHost
                    Node { flex_grow: 1.0, min_height: px(0) }
                    Children [
                        @kit::canvas() EditorCanvasMarker
                        --
                        EmptyHint
                        Node { position_type: PositionType::Absolute, left: px(24), top: px(24) }
                        Pickable::IGNORE
                        Children [ @text("Open an asset from the browser (or create one with +Pbr / +Unlit / +2D / +Fn).", TEXT_DIM, 14.0) ]
                        --
                        @crate::ui::input::minimap()
                    ]
                    --
                    Node { height: px(190), flex_direction: FlexDirection::Column, border: UiRect::top(px(1)) }
                    BackgroundColor(PANEL_BG)
                    BorderColor::all(BORDER)
                    Children [
                        Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, padding: UiRect::axes(px(8), px(2)), column_gap: px(8) }
                        BackgroundColor(HEADER_BG)
                        Children [
                            log::LogHeader Text("Log") TextFont { font_size: FontSize::Px(13.0) } TextColor(TEXT)
                            --
                            Node { flex_grow: 1.0 }
                            --
                            @tool_button("Clear messages", Action::ClearMessages)
                        ]
                        --
                        @scroll_column() log::LogBody
                    ]
                ]
                --
                // Properties.
                Node { width: px(330), flex_direction: FlexDirection::Column, border: UiRect::left(px(1)) }
                BackgroundColor(PANEL_BG)
                BorderColor::all(BORDER)
                Children [
                    @header("Properties")
                    --
                    @scroll_column() props::PropsBody Node { padding: UiRect::all(px(8)), row_gap: px(6) }
                ]
            ]
        ]
    });
}

fn show_empty_hint(editor: Res<Editor>, mut q: Query<&mut Visibility, With<EmptyHint>>) {
    let want = if editor.doc().is_none() { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut q {
        v.set_if_neq(want);
    }
}

fn show_status(editor: Res<Editor>, mut q: Query<&mut Text, With<StatusText>>, mut windows: Query<&mut Window>) {
    if !editor.is_changed() {
        return;
    }
    let status = match &editor.project {
        Some(p) => {
            let errors = editor.all_diagnostics().iter().filter(|d| d.is_error()).count();
            format!("{}  |  {} asset(s)  |  {} error(s)", p.root.display(), editor.disk.assets.len(), errors)
        }
        None => "No project".into(),
    };
    for mut t in &mut q {
        if t.0 != status {
            t.0 = status.clone();
        }
    }
    let title = editor.window_title();
    for mut w in &mut windows {
        if w.title != title {
            w.title = title.clone();
        }
    }
}
