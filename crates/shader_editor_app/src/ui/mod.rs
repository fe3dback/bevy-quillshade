//! Window layout (spec §9, `CLAUDE.md` → "Editor"): menu_bar (top), left sidebar (project tree, assets tree), center
//! column (tabs bar, editor topbar, editor content, editor statusbar, console/logs), right sidebar; the welcome screen
//! covers the panels while no project is open. The editor topbar / statusbar and the right sidebar belong to the
//! active editor kind ([`areas`], [`crate::editors::EditorAreas`]).

use std::path::Path;

use bevy::feathers::controls::{
    ButtonVariant, FeathersButton, FeathersMenu, FeathersMenuToolButton, FeathersToolButton,
};
use bevy::feathers::display::caption;
use bevy::feathers::theme::ThemeBackgroundColor;
use bevy::feathers::tokens;
use bevy::input_focus::tab_navigation::TabGroup;
use bevy::prelude::*;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, MenuButton, ScrollArea};
use bevy_noodle::style::kit;

use crate::config::{Panel, UserConfig};
use crate::state::{Action, Actions, Editor, Unsaved};
use crate::tab::ViewKind;

pub mod areas;
pub mod browser;
pub mod const_preview;
pub mod consts_view;
pub mod curve_preview;
pub mod curve_view;
pub mod dialog;
pub mod icons;
pub mod image_preview;
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
pub mod widgets;

use dialog::menu_separator;
use icons::{IconId, icon};
use tooltip::Tooltip;
use widgets::{pane_box, pane_title};

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

/// The menu_bar's project text (`name  |  N errors`, the full path as its tooltip).
#[derive(Component, Default, Clone)]
pub struct ProjectText;

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
/// Background of the editor topbar / statusbar and of the active tab (which so joins its topbar): darker than
/// [`HEADER_BG`] (the tabs bar), lighter than [`VIEW_BG`].
pub const TOPBAR_BG: Color = Color::srgb_u8(30, 32, 37);
pub const BORDER: Color = Color::srgb_u8(20, 21, 24);
/// Background of document views (the window clear color the graph canvas shows through).
pub const VIEW_BG: Color = Color::srgb_u8(24, 25, 29);

/// Narrowest the center column (editor content and its bars) gets: in a smaller window the sidebars shrink.
const CENTER_MIN_W: f32 = 400.0;

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
            areas::AreasPlugin,
        ))
        .add_plugins((
            logo::LogoPlugin,
            welcome::WelcomePlugin,
            image_preview::ImagePreviewPlugin,
            image_view::ImageViewPlugin,
            texture_view::TextureViewPlugin,
            crate::game_image::GameImagePlugin,
            crate::editors::main_input::InputContextsPlugin,
        ))
        .add_systems(Startup, spawn_layout)
        .add_systems(
            Update,
            (
                show_active_view,
                show_empty_hint,
                show_doc_kind,
                show_project,
                fill_main_menus,
                main_menu_barrier,
                show_left_sidebar,
            ),
        )
        .add_observer(activate_action)
        .add_observer(row_hover_on)
        .add_observer(row_hover_off);
    }
}

/// Pushes the [`OnActivate`] action of an activated widget (not sent for an `InteractionDisabled` one).
pub fn activate_action(ev: On<Activate>, q: Query<&OnActivate>, mut actions: ResMut<Actions>) {
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

/// Small text button that pushes `action`. Buttons never shrink (`flex_shrink: 0`): a caption is not broken inside
/// its button, a row that runs out of room wraps or clips whole buttons instead.
pub fn tool_button(label: impl Into<String>, action: Action) -> impl Scene {
    let label = label.into();
    let act = OnActivate(action);
    bsn! {
        @FeathersToolButton { @variant: ButtonVariant::Plain, @caption: bsn! { @caption(label) } }
        Node { flex_shrink: 0.0 }
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
        Node { flex_shrink: 0.0 }
        act
        tip
    }
}

/// Icon-only tool button; `tip` names the action (and its hotkey).
pub fn icon_button(id: IconId, action: Action, tip: impl Into<String>) -> impl Scene {
    let (act, tip) = (OnActivate(action), Tooltip(tip.into()));
    bsn! {
        @FeathersToolButton { @variant: ButtonVariant::Plain, @caption: bsn! { @icon(id, 16.0, TEXT) } }
        Node { flex_shrink: 0.0 }
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
        Node { flex_shrink: 0.0 }
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

/// `n` and the noun in the right number: `1 node`, `2 nodes`, `0 nodes`.
pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// One line of text that is clipped instead of wrapped when the row is too narrow (a path in an editor topbar).
pub fn clipped_text(s: impl Into<String>, color: Color, font_size: f32) -> impl Scene {
    let (s, layout) = (s.into(), TextLayout::no_wrap());
    bsn! {
        Node { min_width: px(0), overflow: Overflow::clip() }
        Children [ @text(s, color, font_size) layout ]
    }
}

/// A main menu of the menu_bar (on its `FeathersMenu`, see [`main_menu`]).
#[derive(Component, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum MainMenu {
    #[default]
    File,
    Edit,
    View,
    Assets,
    Help,
}

impl MainMenu {
    /// The menus in the menu_bar, left to right.
    pub const ORDER: [MainMenu; 5] = [MainMenu::File, MainMenu::Edit, MainMenu::View, MainMenu::Assets, MainMenu::Help];

    pub fn label(self) -> &'static str {
        match self {
            MainMenu::File => "File",
            MainMenu::Edit => "Edit",
            MainMenu::View => "View",
            MainMenu::Assets => "Assets",
            MainMenu::Help => "Help",
        }
    }

    /// The menu's items; `config` checks the View toggles that are on.
    pub fn items(self, config: &UserConfig) -> Vec<(Option<IconId>, String, Action)> {
        let check = |on: bool| on.then_some(IconId::Check);
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
            MainMenu::Edit => vec![
                (Some(IconId::Undo), "Undo\tCtrl+Z".into(), Action::Undo),
                (Some(IconId::Redo), "Redo\tCtrl+Shift+Z".into(), Action::Redo),
            ],
            // Design §3: each panel's mark is its own flag, also while Zen mode hides the panel.
            MainMenu::View => vec![
                (check(config.zen_mode), "Zen mode\t`".into(), Action::ToggleZen),
                menu_separator(),
                (check(config.show_left_sidebar), "Show project\tCtrl+1".into(), Action::ToggleLeftSidebar),
                (check(config.show_log), "Show console\tCtrl+2".into(), Action::ToggleLog),
                (check(config.show_properties), "Show properties\tCtrl+3".into(), Action::ToggleProperties),
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

/// The popup of a main menu, filled with its items by [`fill_main_menus`].
#[derive(Component, Default, Clone, Copy)]
pub struct MainMenuPopup(pub MainMenu);

/// A main menu: a `FeathersMenu` holding a plain menu tool button (no chevron) and its popup, which opens under the
/// button. Feathers opens and closes it; [`fill_main_menus`] fills it.
pub fn main_menu(menu: MainMenu) -> impl Scene {
    let (label, popup) = (menu.label(), MainMenuPopup(menu));
    bsn! {
        @FeathersMenu
        menu
        Children [
            @FeathersMenuToolButton { @caption: bsn! { @caption(label) }, @arrow: false }
            ButtonVariant::Plain
            --
            @dialog::menu_popup(Vec::new())
            popup
        ]
    }
}

/// The [`dialog::menu_barrier`] of an open main menu: a click outside the popup (the menu_bar's buttons included) only
/// closes the menu, as with a context menu.
#[derive(Component, Default, Clone)]
pub struct MainMenuBarrier;

/// Spawns the [`MainMenuBarrier`] while a main menu's popup is shown and removes it once none is, and gives each main
/// menu button Feathers' active look (`Pressed`) exactly while its popup is shown.
///
/// The button gets `Pressed` from the press that opens the menu (`ActivateOnPress`) and loses it only on a release
/// over itself. That release lands on the barrier, spawned under the pointer in the meantime, so the button would keep
/// looking active after the menu closed and ignore its next press (design §1). Following the popup's visibility here
/// covers every way the menu closes: an outside click, Esc, an item, another menu.
pub fn main_menu_barrier(
    popups: Query<(&Visibility, &ChildOf), With<MainMenuPopup>>,
    children: Query<&Children>,
    buttons: Query<Has<Pressed>, With<MenuButton>>,
    barriers: Query<Entity, With<MainMenuBarrier>>,
    mut commands: Commands,
) {
    let mut open = false;
    for (visibility, menu) in &popups {
        let shown = *visibility == Visibility::Visible;
        open |= shown;
        for button in children.get(menu.parent()).into_iter().flatten() {
            match buttons.get(*button) {
                Ok(false) if shown => {
                    commands.entity(*button).try_insert(Pressed);
                }
                Ok(true) if !shown => {
                    commands.entity(*button).try_remove::<Pressed>();
                }
                _ => {}
            }
        }
    }
    if open && barriers.is_empty() {
        commands.spawn_scene(bsn! { @dialog::menu_barrier() MainMenuBarrier });
    } else if !open {
        for b in &barriers {
            commands.entity(b).despawn();
        }
    }
}

/// (Re)fills the main menus' popups: when spawned and when a View toggle changed (its check mark).
pub fn fill_main_menus(
    editor: Res<Editor>,
    popups: Query<(Entity, &MainMenuPopup)>,
    added: Query<(), Added<MainMenuPopup>>,
    mut last: Local<Option<[bool; 4]>>,
    mut commands: Commands,
) {
    let c = &editor.config;
    let key = [c.zen_mode, c.show_left_sidebar, c.show_log, c.show_properties];
    if *last == Some(key) && added.is_empty() {
        return;
    }
    *last = Some(key);
    for (popup, MainMenuPopup(menu)) in &popups {
        commands.entity(popup).despawn_children();
        let entries = menu.items(&editor.config).into_iter().map(dialog::MenuEntry::from).collect();
        for row in dialog::menu_rows(entries) {
            commands.spawn_scene(bsn! { @{row} ChildOf(popup) });
        }
    }
}

/// Logo, main menu, undo/redo, then the project name and error count. Nothing of one editor kind (that is the editor
/// topbar's).
fn menu_bar() -> impl Scene {
    let tip = Tooltip::default();
    let menus: Vec<Box<dyn Scene>> =
        MainMenu::ORDER.into_iter().map(|m| Box::new(main_menu(m)) as Box<dyn Scene>).collect();
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
            {menus}
            --
            @icon_button(IconId::Undo, Action::Undo, "Undo (Ctrl+Z)")
            --
            @icon_button(IconId::Redo, Action::Redo, "Redo (Ctrl+Shift+Z, Ctrl+Y)")
            --
            Node { flex_grow: 1.0 }
            --
            ProjectText
            Text("")
            TextFont { font_size: FontSize::Px(12.0) }
            TextColor(TEXT_DIM)
            tip
            Node { margin: UiRect::right(px(6)) }
        ]
    }
}

/// The left sidebar (project tree and assets tree), shown by [`show_left_sidebar`].
#[derive(Component, Default, Clone)]
pub struct LeftSidebar;

/// The left sidebar: the project tree and the assets tree, a pane each. `min_width: 0`: below `CENTER_MIN_W` it
/// shrinks past its content (the rows clip) instead of pushing the window wider.
pub fn left_sidebar() -> impl Scene {
    bsn! {
        LeftSidebar
        Node {
            width: px(270),
            min_width: px(0),
            flex_direction: FlexDirection::Column,
            row_gap: px(4),
            padding: UiRect::all(px(4)),
            border: UiRect::right(px(1)),
        }
        BackgroundColor(PANEL_BG)
        BorderColor::all(BORDER)
        Children [
            @pane_box(pane_title("Project"), Vec::new(), bsn! { @scroll_column() browser::BrowserBody })
            Node { flex_grow: 3.0, flex_basis: px(0), min_height: px(0) }
            --
            @pane_box(pane_title("Game Assets"), Vec::new(), bsn! { @scroll_column() browser::GameAssetsBody })
            Node { flex_grow: 2.0, flex_basis: px(0), min_height: px(0) }
        ]
    }
}

/// Follows [`Panel::LeftSidebar`] (View → Show project, Zen mode): hidden, the left sidebar takes no space and Tab
/// skips it ([`areas::display`]). Runs when the flag changed or the sidebar was spawned.
pub fn show_left_sidebar(
    editor: Res<Editor>,
    mut last: Local<Option<bool>>,
    spawned: Query<(), Added<LeftSidebar>>,
    mut sidebars: Query<(&mut Node, &mut Visibility), With<LeftSidebar>>,
) {
    let shown = editor.config.panel_visible(Panel::LeftSidebar);
    if *last == Some(shown) && spawned.is_empty() {
        return;
    }
    *last = Some(shown);
    for (mut node, mut visibility) in &mut sidebars {
        areas::display(&mut node, &mut visibility, shown);
    }
}

/// The project's name: the root directory's name (the whole path when it has none, e.g. `/`).
pub(crate) fn project_name(root: &Path) -> String {
    root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Scrollable column used by the panels; `marker` tags the entity the panel fills. Rows wider than the column (a
/// sidebar narrowed by a small window) are clipped, not drawn over the neighbouring area.
pub fn scroll_column() -> impl Scene {
    bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            min_height: px(0),
            overflow: Overflow { x: OverflowAxis::Clip, y: OverflowAxis::Scroll },
            padding: UiRect::all(px(4)),
            row_gap: px(1),
        }
        ScrollArea
    }
}

fn spawn_layout(mut commands: Commands) {
    commands.spawn(Camera2d);
    commands.spawn_scene(bsn! {
        // `min_width` / `min_height: 0`: Bevy lays a UI root out in an implicit grid cell, which would otherwise grow
        // to the root's min-content size (the editor topbar's buttons do not wrap; the sidebars' panes) and push the
        // right sidebar and the console/logs out of the window.
        Node {
            width: percent(100),
            height: percent(100),
            min_width: px(0),
            min_height: px(0),
            flex_direction: FlexDirection::Column,
        }
        ThemeBackgroundColor(tokens::WINDOW_BG)
        TabGroup
        Children [
            @menu_bar()
            --
            Node { flex_grow: 1.0, flex_direction: FlexDirection::Row, min_height: px(0) }
            Children [
                @left_sidebar()
                --
                // Center: tabs bar, editor topbar, editor content, editor statusbar, console/logs. `flex_basis: 0`
                // here and on the editor content: they take what is left, so the sidebars keep their width and the
                // log its height (with a content-sized basis every sibling shrinks with them). Below `CENTER_MIN_W`
                // the sidebars give way instead.
                Node {
                    flex_grow: 1.0,
                    flex_basis: px(0),
                    flex_direction: FlexDirection::Column,
                    min_width: px(CENTER_MIN_W),
                }
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
                    @areas::editor_topbar()
                    --
                    DocArea
                    Node { flex_grow: 1.0, flex_basis: px(0), min_height: px(0) }
                    Children [
                        @kit::canvas() EditorCanvasMarker ViewOf(ViewKind::Graph)
                        // Dimmer than bevy_noodle's default (design §4.9).
                        bevy_noodle::style::CanvasGrid {
                            minor: Color::srgba(1.0, 1.0, 1.0, 0.006),
                            major: Color::srgba(1.0, 1.0, 1.0, 0.014),
                        }
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
                        // `right` bounds the width: the hint wraps instead of running past the editor content.
                        Node { position_type: PositionType::Absolute, left: px(24), right: px(24), top: px(24) }
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
                    @areas::editor_statusbar()
                    --
                    @log::console()
                ]
                --
                @areas::right_sidebar()
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

/// Watermark text: the asset type of a graph, the language of a generated file ([`crate::doc_kind::TabDoc::watermark`]).
fn show_doc_kind(editor: Res<Editor>, mut q: Query<&mut Text, With<DocKindText>>) {
    let label = editor.tab().and_then(|t| t.doc().watermark()).unwrap_or("");
    for mut t in &mut q {
        if t.0 != label {
            t.0 = label.into();
        }
    }
}

/// The menu_bar's project text and tooltip, and the window title. Keyed like the editor statusbar (`areas`): the log
/// revision (diagnostics; opening or closing a project logs), the tabs revision (tab titles, saves), the active tab
/// and its document's revision (the dirty mark). Not on every change of `Editor`: the error count clones all
/// diagnostics and the dirty mark serialises a graph.
fn show_project(
    editor: Res<Editor>,
    mut q: Query<(&mut Text, &mut Tooltip), With<ProjectText>>,
    spawned: Query<(), Added<ProjectText>>,
    mut windows: Query<&mut Window>,
    mut last: Local<Option<(u64, u64, Option<usize>, u64)>>,
) {
    let key = (editor.ui.log, editor.ui.tabs, editor.active, editor.tab().map_or(0, |t| t.doc().revision()));
    if *last == Some(key) && spawned.is_empty() {
        return;
    }
    *last = Some(key);
    let (status, path) = match &editor.project {
        Some(p) => {
            let errors = editor.all_diagnostics().iter().filter(|d| d.is_error()).count();
            (
                format!("{}  |  {}", project_name(&p.root), plural(errors, "error", "errors")),
                p.root.display().to_string(),
            )
        }
        None => ("No project".into(), "No project open".into()),
    };
    for (mut t, mut tip) in &mut q {
        if t.0 != status {
            t.0 = status.clone();
        }
        if tip.0 != path {
            tip.0 = path.clone();
        }
    }
    let title = editor.window_title();
    for mut w in &mut windows {
        if w.title != title {
            w.title = title.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::icons::IconId;
    use super::{MainMenu, plural, project_name};
    use crate::config::UserConfig;
    use crate::state::Action;

    #[test]
    fn plural_words() {
        assert_eq!(plural(1, "node", "nodes"), "1 node");
        assert_eq!(plural(2, "node", "nodes"), "2 nodes");
        assert_eq!(plural(0, "node", "nodes"), "0 nodes");
    }

    /// Design §3: the View menu lists Zen mode, a separator and the three panels with their hotkeys; each check mark
    /// is its own flag (Zen on does not clear the panels' marks); Edit sits between File and View.
    #[test]
    fn view_menu_check_marks_follow_config() {
        let marks = |zen_mode, show_left_sidebar, show_log, show_properties| {
            let config = UserConfig { zen_mode, show_left_sidebar, show_log, show_properties, ..UserConfig::default() };
            MainMenu::View.items(&config)
        };
        let check = Some(IconId::Check);
        assert_eq!(
            marks(false, true, true, true),
            [
                (None, "Zen mode\t`".to_string(), Action::ToggleZen),
                (None, String::new(), Action::None),
                (check, "Show project\tCtrl+1".to_string(), Action::ToggleLeftSidebar),
                (check, "Show console\tCtrl+2".to_string(), Action::ToggleLog),
                (check, "Show properties\tCtrl+3".to_string(), Action::ToggleProperties),
            ]
        );
        let icons = |m: Vec<(Option<IconId>, String, Action)>| m.into_iter().map(|i| i.0).collect::<Vec<_>>();
        assert_eq!(icons(marks(true, true, true, true)), [check, None, check, check, check]);
        assert_eq!(icons(marks(true, false, true, false)), [check, None, None, check, None]);
        assert_eq!(icons(marks(false, true, false, true)), [None, None, check, None, check]);

        let edit = MainMenu::Edit.items(&UserConfig::default());
        let edit: Vec<_> = edit.into_iter().map(|(_, label, action)| (label, action)).collect();
        assert_eq!(
            edit,
            [("Undo\tCtrl+Z".to_string(), Action::Undo), ("Redo\tCtrl+Shift+Z".to_string(), Action::Redo)]
        );
        assert_eq!(&MainMenu::ORDER[..3], [MainMenu::File, MainMenu::Edit, MainMenu::View]);
    }

    #[test]
    fn project_name_is_last_component() {
        assert_eq!(project_name(Path::new("/home/u/games/my_game")), "my_game");
        assert_eq!(project_name(Path::new("/")), "/");
    }
}
