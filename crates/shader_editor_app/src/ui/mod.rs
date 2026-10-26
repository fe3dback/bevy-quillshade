//! Window layout (spec §9): main menu + toolbar, asset browser (left), tabs + canvas + log (center), properties
//! (right); the welcome screen covers the panels while no project is open.

use bevy::feathers::constants::size;
use bevy::feathers::controls::{ButtonVariant, FeathersButton, FeathersToolButton};
use bevy::feathers::display::caption;
use bevy::feathers::theme::ThemeBackgroundColor;
use bevy::feathers::tokens;
use bevy::input_focus::tab_navigation::TabGroup;
use bevy::prelude::*;
use bevy::ui::ui_transform::UiGlobalTransform;
use bevy::ui_widgets::{Activate, Button, ScrollArea};
use bevy_noodle::style::kit;

use crate::state::{Action, Actions, Editor, GenFile, Unsaved, ViewRequest};
use crate::tab::{Lang, Tab, ViewKind};

pub mod browser;
pub mod const_preview;
pub mod consts_view;
pub mod curve_preview;
pub mod curve_view;
pub mod dialog;
pub mod icons;
pub mod image_view;
pub mod input;
pub mod log;
pub mod logo;
pub mod node_preview;
pub mod palette;
pub mod props;
pub mod tabs;
pub mod text_view;
pub mod texture_preview;
pub mod texture_view;
pub mod tooltip;
pub mod tree;
pub mod welcome;

use dialog::{menu_separator, open_icon_menu};
use icons::{IconId, icon};
use tooltip::Tooltip;

/// Pushes its action when the widget is activated (buttons, menu items).
#[derive(Component, Default, Clone)]
pub struct OnActivate(pub Action);

/// The area the active tab's view fills (below the tab bar).
#[derive(Component, Default, Clone)]
pub struct DocArea;

/// A view in [`DocArea`]: shown only while the active tab is of its kind (the graph view also when no tab is
/// open). Every view fills the area with a dark background; the [`DocKindText`] watermark stays on top.
#[derive(Component, Clone)]
pub struct ViewOf(pub ViewKind);

impl Default for ViewOf {
    fn default() -> Self {
        ViewOf(ViewKind::Graph)
    }
}

#[derive(Component, Default, Clone)]
pub struct EditorCanvasMarker;

#[derive(Component, Default, Clone)]
pub struct EmptyHint;

#[derive(Component, Default, Clone)]
pub struct StatusText;

/// Watermark in the main view's bottom-right corner naming the open document's type.
#[derive(Component, Default, Clone)]
pub struct DocKindText;

/// The UI accent (orange): the accent detail of icons, markers of unsaved changes. The only definition of it
/// (`icons` builds its icon palette from [`ACCENT_RGB`]).
pub const ACCENT_RGB: [u8; 3] = [0xfb, 0x92, 0x3c];
pub const ACCENT: Color = Color::srgb_u8(ACCENT_RGB[0], ACCENT_RGB[1], ACCENT_RGB[2]);
pub const TEXT: Color = Color::srgb(0.86, 0.87, 0.90);
pub const TEXT_DIM: Color = Color::srgb(0.55, 0.57, 0.62);
pub const ROW_HOVER: Color = Color::srgba(1.0, 1.0, 1.0, 0.06);
pub const ROW_ACTIVE: Color = Color::srgba(0.35, 0.55, 0.95, 0.30);
pub const PANEL_BG: Color = Color::srgb_u8(32, 34, 39);
pub const HEADER_BG: Color = Color::srgb_u8(40, 42, 48);
pub const BORDER: Color = Color::srgb_u8(20, 21, 24);
/// Background of document views (the window clear color the graph canvas shows through).
pub const VIEW_BG: Color = Color::srgb_u8(24, 25, 29);

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            browser::BrowserPlugin,
            tabs::TabsPlugin,
            text_view::TextViewPlugin,
            curve_view::CurveViewPlugin,
            consts_view::ConstsViewPlugin,
            node_preview::NodePreviewsPlugin,
            props::PropsPlugin,
            log::LogPlugin,
            dialog::DialogPlugin,
            icons::IconsPlugin,
            palette::PalettePlugin,
            input::InputPlugin,
            tooltip::TooltipPlugin,
        ))
        .add_plugins((
            logo::LogoPlugin,
            welcome::WelcomePlugin,
            image_view::ImageViewPlugin,
            texture_view::TextureViewPlugin,
            crate::game_image::GameImagePlugin,
        ))
        .add_systems(Startup, spawn_layout)
        .add_systems(Update, (show_active_view, show_empty_hint, show_doc_kind, show_status, redraw_view_tools))
        .add_observer(activate_action)
        .add_observer(open_main_menu)
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

/// [`tool_button`] with an icon before the label and a hover tooltip.
pub fn icon_tool_button(id: IconId, label: impl Into<String>, action: Action, tip: impl Into<String>) -> impl Scene {
    let (label, tip) = (label.into(), Tooltip(tip.into()));
    let act = OnActivate(action);
    bsn! {
        @FeathersToolButton {
            @variant: ButtonVariant::Plain,
            @caption: bsn_list! { @icon(id, 16.0, TEXT) Node { margin: UiRect::right(px(4)) } -- @caption(label) },
        }
        act
        tip
    }
}

/// Icon-only tool button; `tip` names the action (and its hotkey).
pub fn icon_button(id: IconId, action: Action, tip: impl Into<String>) -> impl Scene {
    let (act, tip) = (OnActivate(action), Tooltip(tip.into()));
    bsn! {
        @FeathersToolButton { @variant: ButtonVariant::Plain, @caption: bsn! { @icon(id, 16.0, TEXT) } }
        act
        tip
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

/// A toggle chip (a step tab, one button of a button group): accent background while `selected`, an optional dim
/// `tag` after the label.
pub fn chip(
    label: impl Into<String>,
    action: Action,
    selected: bool,
    tip: impl Into<String>,
    tag: Option<&str>,
) -> impl Scene {
    let label = label.into();
    let base = if selected { ACCENT.with_alpha(0.45) } else { Color::NONE };
    let (act, hover, tip) = (OnActivate(action), HoverRow { base }, Tooltip(tip.into()));
    let tag_color = if selected { TEXT } else { TEXT_DIM };
    let tags: Vec<Box<dyn Scene>> =
        tag.map(|t| -> Box<dyn Scene> { Box::new(text(t, tag_color, 11.0)) }).into_iter().collect();
    bsn! {
        Node {
            align_items: AlignItems::Center,
            column_gap: px(6),
            padding: UiRect::axes(px(8), px(4)),
            border_radius: BorderRadius::all(px(4)),
        }
        BackgroundColor(base)
        Button
        act
        hover
        tip
        Children [ @text(label, TEXT, 13.0) -- {tags} ]
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

/// A main menu button; activating it opens the menu below it.
#[derive(Component, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum MainMenu {
    #[default]
    File,
    Assets,
    Help,
}

impl MainMenu {
    pub fn items(self) -> Vec<(Option<IconId>, String, Action)> {
        match self {
            MainMenu::File => vec![
                (Some(IconId::FolderOpen), "Open Project...\tCtrl+O".into(), Action::OpenProjectDialog),
                (Some(IconId::FolderNew), "New Project...".into(), Action::NewProjectDialog),
                (Some(IconId::Close), "Close Project".into(), Action::CloseProject(Unsaved::Ask)),
                menu_separator(),
                (Some(IconId::Save), "Save\tCtrl+S".into(), Action::Save),
                (Some(IconId::SaveAll), "Save All\tCtrl+Shift+S".into(), Action::SaveAll),
                menu_separator(),
                (None, "Quit\tCtrl+Q".into(), Action::Quit),
            ],
            MainMenu::Assets => {
                let mut items = browser::create_items();
                items.push(menu_separator());
                items.push((Some(IconId::Regenerate), "Regenerate".into(), Action::RegenAll));
                items
            }
            MainMenu::Help => vec![
                (Some(IconId::Help), "Quick Start".into(), Action::QuickStart(0)),
                (Some(IconId::Link), "Project Page".into(), Action::OpenUrl(crate::PROJECT_URL.into())),
                menu_separator(),
                (Some(IconId::Info), "About".into(), Action::About),
            ],
        }
    }
}

fn menu_button(label: &'static str, menu: MainMenu) -> impl Scene {
    bsn! {
        @FeathersToolButton { @variant: ButtonVariant::Plain, @caption: bsn! { @caption(label) } }
        menu
    }
}

fn open_main_menu(
    ev: On<Activate>,
    buttons: Query<(&MainMenu, &ComputedNode, &UiGlobalTransform)>,
    mut commands: Commands,
) {
    let Ok((menu, node, t)) = buttons.get(ev.event_target()) else { return };
    let (center, size) = (t.translation * node.inverse_scale_factor(), node.size() * node.inverse_scale_factor());
    open_icon_menu(&mut commands, center + Vec2::new(-size.x, size.y) / 2.0 + Vec2::Y * 2.0, menu.items());
}

/// The toolbar part with the actions of the active tab's view (filled by [`redraw_view_tools`]).
#[derive(Component, Default, Clone)]
pub struct ViewTools;

/// Logo, main menu, undo/redo, then the actions of the active view.
fn toolbar() -> impl Scene {
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
            @logo::logo(20.0) Node { margin: UiRect::axes(px(4), px(0)) }
            --
            @menu_button("File", MainMenu::File)
            --
            @menu_button("Assets", MainMenu::Assets)
            --
            @menu_button("Help", MainMenu::Help)
            --
            @icon_button(IconId::Undo, Action::Undo, "Undo (Ctrl+Z)")
            --
            @icon_button(IconId::Redo, Action::Redo, "Redo (Ctrl+Shift+Z, Ctrl+Y)")
            --
            @sep()
            --
            ViewTools
            Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, column_gap: px(2) }
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

/// Toolbar actions of the active tab's view.
fn view_tools(tab: &Tab) -> Vec<Box<dyn Scene>> {
    match tab {
        Tab::Graph(_) => vec![
            Box::new(icon_tool_button(IconId::AutoLayout, "Auto-layout", Action::AutoLayout, "Auto-layout (Ctrl+L)")),
            Box::new(icon_tool_button(IconId::Comment, "Comment", Action::AddComment(None), "Add a comment frame (C)")),
            Box::new(icon_tool_button(IconId::Fit, "Fit", Action::View(ViewRequest::FitAll), "Fit the graph (Home)")),
        ],
        Tab::Curve(_) => vec![
            Box::new(tool_button("Select all", Action::SelectAll)),
            Box::new(icon_tool_button(
                IconId::Delete,
                "Delete",
                Action::DeleteSelection,
                "Delete the selected points (Del)",
            )),
        ],
        // The add form has its own Add button; an image has nothing to act on (Properties offers its Texture asset);
        // a Texture tab's form and preview controls are in the view.
        Tab::Consts(_) | Tab::Image(_) | Tab::Texture(_) => Vec::new(),
        Tab::Text(t) => {
            let which = match t.lang {
                Lang::Wesl => GenFile::Wesl,
                Lang::Rust => GenFile::Rust,
            };
            vec![
                Box::new(icon_tool_button(IconId::Copy, "Copy", Action::Copy, "Copy the whole file (Ctrl+C)")),
                Box::new(icon_tool_button(
                    IconId::for_id(&t.source, None),
                    "Source",
                    Action::OpenAsset(t.source.clone()),
                    "Open the source graph",
                )),
                Box::new(icon_tool_button(
                    IconId::OpenExternal,
                    "Open externally",
                    Action::ShowFile(t.source.clone(), which),
                    "Open in the default application",
                )),
            ]
        }
    }
}

fn redraw_view_tools(
    editor: Res<Editor>,
    tools: Query<Entity, With<ViewTools>>,
    mut last: Local<Option<Option<String>>>,
    mut commands: Commands,
) {
    // A text tab's buttons name its source: rebuilt per tab, not per view kind.
    let key = editor.tab().and_then(|t| t.doc().config_key());
    if last.as_ref() == Some(&key) {
        return;
    }
    let Ok(tools) = tools.single() else { return };
    *last = Some(key);
    commands.entity(tools).despawn_children();
    for b in editor.tab().map(view_tools).unwrap_or_default() {
        commands.spawn_scene(bsn! { @{b} ChildOf(tools) });
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
                    @header("Project")
                    --
                    @scroll_column() browser::BrowserBody Node { flex_grow: 3.0, flex_basis: px(0) }
                    --
                    @header("Game Assets")
                    --
                    @scroll_column() browser::GameAssetsBody Node { flex_grow: 2.0, flex_basis: px(0) }
                ]
                --
                // Center: tabs, canvas, log.
                Node { flex_grow: 1.0, flex_direction: FlexDirection::Column, min_width: px(0) }
                Children [
                    tabs::TabsBar
                    Node {
                        flex_direction: FlexDirection::Row,
                        flex_wrap: FlexWrap::Wrap,
                        min_height: px(24),
                        align_items: AlignItems::End,
                        column_gap: px(1),
                        row_gap: px(1),
                        padding: UiRect::new(px(4), px(4), px(3), px(0)),
                    }
                    BackgroundColor(HEADER_BG)
                    --
                    DocArea
                    Node { flex_grow: 1.0, min_height: px(0) }
                    Children [
                        @kit::canvas() EditorCanvasMarker ViewOf(ViewKind::Graph)
                        --
                        @text_view::text_view()
                        --
                        @curve_view::curve_view()
                        --
                        @consts_view::consts_view()
                        --
                        @image_view::image_view()
                        --
                        @texture_view::texture_view()
                        --
                        EmptyHint
                        Node { position_type: PositionType::Absolute, left: px(24), top: px(24) }
                        Pickable::IGNORE
                        Children [ @text("Double-click an asset in the browser to open it (create one with Assets or a right click in the browser).", TEXT_DIM, 14.0) ]
                        --
                        // Over the active view, whatever kind it is.
                        DocKindText
                        Text("")
                        TextFont { font_size: FontSize::Px(72.0), weight: FontWeight::BOLD }
                        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.1))
                        Node { position_type: PositionType::Absolute, right: px(20), bottom: px(8) }
                        Pickable::IGNORE
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
                --
                @welcome::welcome()
            ]
        ]
    });
}

fn show_active_view(editor: Res<Editor>, mut q: Query<(&ViewOf, &mut Visibility)>) {
    let active = editor.tab().map_or(ViewKind::Graph, |t| t.doc().view());
    for (view, mut v) in &mut q {
        v.set_if_neq(if view.0 == active { Visibility::Inherited } else { Visibility::Hidden });
    }
}

fn show_empty_hint(editor: Res<Editor>, mut q: Query<&mut Visibility, With<EmptyHint>>) {
    let want = if editor.tab().is_none() { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut q {
        v.set_if_neq(want);
    }
}

/// Watermark text: the asset type of a graph, the language of a generated file ([`crate::doc_kind::TabDoc::kind_label`]).
fn show_doc_kind(editor: Res<Editor>, mut q: Query<&mut Text, With<DocKindText>>) {
    let label = editor.tab().map_or("", |t| t.doc().kind_label());
    for mut t in &mut q {
        if t.0 != label {
            t.0 = label.into();
        }
    }
}

fn show_status(editor: Res<Editor>, mut q: Query<&mut Text, With<StatusText>>, mut windows: Query<&mut Window>) {
    if !editor.is_changed() {
        return;
    }
    let status = match &editor.project {
        Some(p) => {
            let errors = editor.all_diagnostics().iter().filter(|d| d.is_error()).count();
            format!("{}  |  {} asset(s)  |  {} error(s)", p.root.display(), editor.disk.len(), errors)
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
