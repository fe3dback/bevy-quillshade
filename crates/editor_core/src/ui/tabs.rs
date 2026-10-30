//! The domain tabs and the secondbar (design §14.3; spec §9 "State").
//!
//! - **Domain tabs** ([`DomainTabs`]): in the mainbar after the main menu, Blender style, one custom button per
//!   domain (the project domain last) showing only its name, the active one highlighted in [`DOMAIN_TAB_ACTIVE_BG`]
//!   with [`DOMAIN_TAB_ACTIVE_TEXT`] ([`restyle_domain_tabs`]).
//! - **Secondbar** ([`SecondBar`]): the whole window's width under the mainbar, `{main actions} | {domain actions} |
//!   {tabs}`. The main actions ([`MainActions`]) are Back / Forward; the domain actions ([`DomainActions`]) reserve a
//!   slot that nothing fills yet; the open tabs of every domain wrap in [`TabRow`], one tab row high, growing by
//!   whole rows rather than scrolling so no tab is ever hidden. Tabs are compact, labelled with the short file name
//!   ([`labels`]); a tool tab has no icon and a blue background, a tab of a real domain other than the active one is
//!   greyed ([`TabLook`]). MMB closes a tab; RMB opens its close menu ([`tab_click`]).

use crate::action::{Action, Actions, CloseTabs, Unsaved};
use crate::tab::labels;
use crate::theme::{
    BORDER, DOMAIN_TAB_ACTIVE_BG, DOMAIN_TAB_ACTIVE_TEXT, DOMAIN_TAB_DISABLED_TEXT, DOMAIN_TAB_TEXT, ROW_HOVER, TAB_BG,
    TAB_GREY, TEXT, TEXT_DIM, TOOL_TAB_ACTIVE_BG, TOOL_TAB_BG, TOPBAR_BG, text,
};
use crate::tooltip::Tooltip;
use crate::ui::HoverRow;
use crate::ui::icons::{IconId, IconRef, icon};
use bevy::feathers::constants::{fonts, size};
use bevy::feathers::controls::{ButtonVariant, FeathersToolButton};
use bevy::feathers::focus::FocusIndicator;
use bevy::input_focus::tab_navigation::TabIndex;
use bevy::picking::cursor::EntityCursor;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::FontSourceTemplate;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::Button;
use bevy::window::SystemCursorIcon;
use project_core::DomainId;

use super::OnActivate;
use super::dialog::open_icon_menu;
use super::icon_button;
use super::shell::{HEADER_BG, MainBarSeparator, NavButton};
use crate::state::editor::Editor;

/// The secondbar: [`MainActions`], [`DomainActions`], then [`TabRow`].
#[derive(Component, Default, Clone)]
pub struct SecondBar;

/// The secondbar's main actions: Back / Forward ([`NavButton`], decision 0041).
#[derive(Component, Default, Clone)]
pub struct MainActions;

/// The secondbar's slot for a domain's main actions. Nothing fills it yet, so it is hidden and takes no space; its
/// separator (the right border) shows only with it.
#[derive(Component, Default, Clone)]
pub struct DomainActions;

/// The domain tabs: the mainbar's part after the main menu, filled by [`spawn_domain_tabs`] and restyled by
/// [`restyle_domain_tabs`]; hidden without a project ([`show_domain_tabs`]).
#[derive(Component, Default, Clone)]
pub struct DomainTabs;

/// A domain tab: activating it switches to its domain (`Action::SwitchDomain`).
#[derive(Component, Clone)]
pub struct DomainTab(pub DomainId);

/// The wrapping row of the open tabs, refilled by [`redraw`].
#[derive(Component, Default, Clone)]
pub struct TabRow;

/// Tab index on a tab entity (middle click closes, right click opens the menu).
#[derive(Component, Default, Clone)]
pub struct TabIndexOf(pub usize);

/// Height of one tab: every tab has it, so the [`TabRow`] grows by whole rows.
pub const TAB_H: f32 = 22.0;
/// The gap between two rows of tabs.
pub const ROW_GAP: f32 = 1.0;
const ROW_PAD_TOP: f32 = 3.0;
/// The [`TabRow`]'s height with one row of tabs, and its minimum (design §14.3: the secondbar is one tab row high).
pub const TAB_ROW_MIN_H: f32 = ROW_PAD_TOP + TAB_H;

pub struct TabsPlugin;

impl Plugin for TabsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (redraw, spawn_domain_tabs, restyle_domain_tabs, show_domain_tabs).after(crate::apply_actions),
        );
    }
}

/// The secondbar, filled by [`redraw`]. Its sections are split by their right border.
pub fn second_bar() -> impl Scene {
    bsn! {
        SecondBar
        Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Stretch }
        BackgroundColor(HEADER_BG)
        Children [
            // At the top: with several rows of tabs Back / Forward stay by the first.
            MainActions
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::FlexStart,
                flex_shrink: 0.0,
                column_gap: px(2),
                padding: UiRect::new(px(6), px(4), px(1), px(0)),
                border: UiRect::right(px(1)),
            }
            BorderColor::all(BORDER)
            Children [
                @icon_button(IconId::Back, Action::NavBack, "Back (Ctrl+[, mouse back button)") NavButton(0)
                --
                @icon_button(IconId::Forward, Action::NavForward, "Forward (Ctrl+], mouse forward button)") NavButton(1)
            ]
            --
            DomainActions
            Node {
                display: Display::None,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::FlexStart,
                flex_shrink: 0.0,
                column_gap: px(2),
                padding: UiRect::new(px(4), px(4), px(1), px(0)),
                border: UiRect::right(px(1)),
            }
            BorderColor::all(BORDER)
            --
            // Lines pack at the bottom: a single row of tabs sits on the editor topbar, which the active tab joins.
            TabRow
            Node {
                flex_grow: 1.0,
                flex_basis: px(0),
                min_width: px(0),
                min_height: px(TAB_ROW_MIN_H),
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                align_content: AlignContent::FlexEnd,
                align_items: AlignItems::End,
                column_gap: px(1),
                row_gap: px(ROW_GAP),
                padding: UiRect::new(px(4), px(4), px(ROW_PAD_TOP), px(0)),
            }
        ]
    }
}

/// Tooltip on a disabled domain tab in the mainbar.
pub const DISABLED_DOMAIN_TIP: &str = "Module is disabled, enable it in Project Settings first";

/// The domain tabs' container in the mainbar, filled by [`spawn_domain_tabs`].
pub fn domain_tabs() -> impl Scene {
    bsn! {
        DomainTabs
        Node {
            height: percent(100),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(2),
        }
    }
}

/// The look of a domain tab: a disabled one has a transparent fill with [`DOMAIN_TAB_DISABLED_TEXT`]; the active one
/// has [`DOMAIN_TAB_ACTIVE_BG`] with [`DOMAIN_TAB_ACTIVE_TEXT`], the others a transparent fill with
/// [`DOMAIN_TAB_TEXT`].
fn domain_tab_look(active: bool, enabled: bool) -> (Color, Color) {
    if !enabled {
        (Color::NONE, DOMAIN_TAB_DISABLED_TEXT)
    } else if active {
        (DOMAIN_TAB_ACTIVE_BG, DOMAIN_TAB_ACTIVE_TEXT)
    } else {
        (Color::NONE, DOMAIN_TAB_TEXT)
    }
}

/// One domain tab: a full-height rectangular button with the domain's name and no icon, in [`domain_tab_look`].
fn domain_tab(id: DomainId, title: &'static str, active: bool, enabled: bool) -> impl Scene {
    let (tab, act, name) = (DomainTab(id), OnActivate(Action::SwitchDomain(id)), DomainTabName(id));
    let (bg, ink) = domain_tab_look(active, enabled);
    let hover = HoverRow { base: bg };
    bsn! {
        Node {
            height: percent(100),
            flex_shrink: 0.0,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            padding: UiRect::axes(px(10), px(0)),
            border_radius: BorderRadius::ZERO,
        }
        BackgroundColor(bg)
        Button
        Hovered::default()
        EntityCursor::System(SystemCursorIcon::Pointer)
        TabIndex(0)
        FocusIndicator
        hover
        tab
        act
        Children [ @domain_tab_name(title, ink) name ]
    }
}

/// The name text of a domain's tab, recoloured by [`restyle_domain_tabs`].
#[derive(Component, Clone)]
pub struct DomainTabName(pub DomainId);

/// A domain tab's name: the font of the main menu's captions in a colour of its own.
fn domain_tab_name(title: &'static str, ink: Color) -> impl Scene {
    bsn! {
        Text(title)
        TextFont { font: FontSourceTemplate::Handle(fonts::REGULAR), font_size: size::MEDIUM_FONT }
        TextColor(ink)
        Pickable { should_block_lower: false, is_hoverable: false }
    }
}

/// Fills the domain tabs once, when spawned: one per real domain of the (fixed) domain list.
fn spawn_domain_tabs(editor: Res<Editor>, bars: Query<Entity, Added<DomainTabs>>, mut commands: Commands) {
    for bar in &bars {
        for def in editor.domain_list.domains().iter().filter(|d| d.id != DomainId::PROJECT) {
            let enabled = editor.domain_enabled(def.id);
            let active = enabled && editor.active_domain == Some(def.id);
            let scene = domain_tab(def.id, def.info.title, active, enabled);
            let e = commands.spawn_scene(bsn! { @{scene} ChildOf(bar) }).id();
            if !enabled {
                commands.entity(e).insert((
                    InteractionDisabled,
                    Tooltip(DISABLED_DOMAIN_TIP.into()),
                    EntityCursor::System(SystemCursorIcon::Default),
                ));
            }
        }
    }
}

/// Restyles the domain tabs in place when the active domain or enabled domains changed (or a tab appeared): the same
/// entities, so a domain tab activated from the keyboard keeps the focus.
fn restyle_domain_tabs(
    editor: Res<Editor>,
    added: Query<(), Or<(Added<DomainTab>, Added<DomainTabName>)>>,
    mut tabs: Query<(Entity, &DomainTab, &mut BackgroundColor, &mut HoverRow, Option<&Hovered>)>,
    mut names: Query<(&DomainTabName, &mut TextColor)>,
    mut last: Local<Option<(Option<DomainId>, Vec<(DomainId, bool)>)>>,
    mut commands: Commands,
) {
    let enabled_vec: Vec<(DomainId, bool)> =
        editor.domain_list.real_ids().map(|id| (id, editor.domain_enabled(id))).collect();
    let state = (editor.active_domain, enabled_vec);
    if last.as_ref() == Some(&state) && added.is_empty() {
        return;
    }
    *last = Some(state);
    for (entity, DomainTab(id), mut bg, mut hover, hovered) in &mut tabs {
        let enabled = editor.domain_enabled(*id);
        let active = enabled && editor.active_domain == Some(*id);
        let (base, _) = domain_tab_look(active, enabled);
        hover.base = base;
        let fill = if enabled && hovered.is_some_and(|h| h.0) && base.alpha() < 0.2 { ROW_HOVER } else { base };
        bg.set_if_neq(BackgroundColor(fill));
        if enabled {
            commands
                .entity(entity)
                .remove::<(InteractionDisabled, Tooltip)>()
                .insert(EntityCursor::System(SystemCursorIcon::Pointer));
        } else {
            commands.entity(entity).insert((
                InteractionDisabled,
                Tooltip(DISABLED_DOMAIN_TIP.into()),
                EntityCursor::System(SystemCursorIcon::Default),
            ));
        }
    }
    for (DomainTabName(id), mut color) in &mut names {
        let enabled = editor.domain_enabled(*id);
        let active = enabled && editor.active_domain == Some(*id);
        color.set_if_neq(TextColor(domain_tab_look(active, enabled).1));
    }
}

/// Whether the domain tabs show: only with a project open; without one they would do nothing (design §14.3).
pub fn domain_tabs_shown(editor: &Editor) -> bool {
    editor.project.is_some()
}

/// Hides the domain tabs and the mainbar's separators ([`MainBarSeparator`]) while no project is open
/// ([`domain_tabs_shown`]). Runs when that changed or one was spawned.
fn show_domain_tabs(
    editor: Res<Editor>,
    spawned: Query<(), Or<(Added<DomainTabs>, Added<MainBarSeparator>)>>,
    mut bars: Query<(&mut Node, &mut Visibility), Or<(With<DomainTabs>, With<MainBarSeparator>)>>,
    mut last: Local<Option<bool>>,
) {
    let shown = domain_tabs_shown(&editor);
    if *last == Some(shown) && spawned.is_empty() {
        return;
    }
    *last = Some(shown);
    for (mut node, mut visibility) in &mut bars {
        super::areas::display(&mut node, &mut visibility, shown);
    }
}

/// How a tab is drawn (design §14.3): its background, the colour of its text and icons, and whether it has an icon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabLook {
    pub bg: Color,
    pub ink: Color,
    pub icon: bool,
}

impl TabLook {
    /// A tool tab (`TabDoc::tool`) has no icon and the tool background; the active asset tab has the editor topbar's
    /// colour (it joins the strip of its editor kind below the secondbar); a greyed tab (`Editor::is_greyed`) has
    /// [`TAB_GREY`] text and icon.
    pub fn of(tool: bool, active: bool, greyed: bool) -> TabLook {
        let bg = match (tool, active) {
            (true, true) => TOOL_TAB_ACTIVE_BG,
            (true, false) => TOOL_TAB_BG,
            (false, true) => TOPBAR_BG,
            (false, false) => TAB_BG,
        };
        let ink = if greyed {
            TAB_GREY
        } else if active {
            TEXT
        } else {
            TEXT_DIM
        };
        TabLook { bg, ink, icon: !tool }
    }
}

/// One compact tab: icon (none for a tool), label (the full path is its tooltip) and a close cross.
fn tab(i: usize, label: String, full: String, kind: IconRef, look: TabLook) -> impl Scene {
    let TabLook { bg: base, ink: color, .. } = look;
    let (hover, act, idx, tip) = (HoverRow { base }, OnActivate(Action::ActivateTab(i)), TabIndexOf(i), Tooltip(full));
    let close = OnActivate(Action::CloseTab(i));
    let close_tip = Tooltip("Close (Ctrl+W, middle click)".into());
    let kind: Vec<Box<dyn Scene>> =
        if look.icon { vec![Box::new(icon(kind, 14.0, color)) as Box<dyn Scene>] } else { Vec::new() };
    bsn! {
        Node {
            height: px(TAB_H),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            padding: UiRect::new(px(6), px(2), px(2), px(2)),
            column_gap: px(4),
            border_radius: BorderRadius::top(px(4)),
        }
        BackgroundColor(base)
        Button
        hover
        act
        idx
        tip
        Children [
            {kind}
            --
            @text(label, color, 12.0)
            --
            // A plain Feathers tool button, only as large as its icon.
            @FeathersToolButton { @variant: ButtonVariant::Plain, @caption: bsn! { @icon(IconId::Close, 12.0, color) } }
            Node { height: Val::Auto, min_width: px(0), padding: UiRect::all(px(2)), border_radius: BorderRadius::all(px(3)) }
            close
            close_tip
        ]
    }
}

type TabKey = (String, String, IconRef, bool, TabLook);

fn redraw(
    editor: Res<Editor>,
    row: Query<Entity, With<TabRow>>,
    mut last: Local<Option<Vec<TabKey>>>,
    mut commands: Commands,
) {
    let labels = labels(&editor.tabs);
    let key: Vec<TabKey> = editor
        .tabs
        .iter()
        .zip(labels)
        .enumerate()
        .map(|(i, (t, label))| {
            let look = TabLook::of(t.doc().tool().is_some(), editor.active == Some(i), editor.is_greyed(i));
            (label, t.title(), t.doc().icon(), t.doc().is_dirty(), look)
        })
        .collect();
    if last.as_ref() == Some(&key) {
        return;
    }
    let Ok(row) = row.single() else { return };
    *last = Some(key.clone());
    commands.entity(row).despawn_children();
    for (i, (label, full, kind, dirty, look)) in key.into_iter().enumerate() {
        let label = if dirty { format!("{label} *") } else { label };
        commands.spawn_scene(bsn! { @tab(i, label, full, kind, look) ChildOf(row) });
    }
}

/// MMB closes the tab, RMB opens its close menu ([`tab_menu`]).
pub fn tab_click(ev: On<PointerClick>, tabs: Query<&TabIndexOf>, mut actions: ResMut<Actions>, mut commands: Commands) {
    let Ok(&TabIndexOf(i)) = tabs.get(ev.event_target()) else { return };
    match ev.button {
        PointerButton::Middle => actions.push(Action::CloseTab(i)),
        PointerButton::Secondary => open_icon_menu(&mut commands, ev.pointer.position, tab_menu(i)),
        PointerButton::Primary => {}
    }
}

/// The close menu of tab `i`.
pub fn tab_menu(i: usize) -> Vec<(Option<IconRef>, String, Action)> {
    let close = |which| Action::CloseTabs { which, at: i, unsaved: Unsaved::Ask };
    vec![
        (Some(IconId::Close.into()), "Close\tCtrl+W".into(), Action::CloseTab(i)),
        (None, "Close All".into(), close(CloseTabs::All)),
        (None, "Close Others".into(), close(CloseTabs::Others)),
        (None, "Close to the Left".into(), close(CloseTabs::Left)),
        (None, "Close to the Right".into(), close(CloseTabs::Right)),
        (None, "Close Unmodified".into(), close(CloseTabs::Unmodified)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ACCENT;

    /// Design §14.3: a tool tab has no icon and a blue background (brighter while active), apart from the asset tabs
    /// and the accent; the active asset tab joins the editor topbar.
    #[test]
    fn tool_tabs_have_no_icon_and_their_own_background() {
        let tool = TabLook::of(true, false, false);
        let active_tool = TabLook::of(true, true, false);
        assert!(!tool.icon && !active_tool.icon);
        assert_eq!((tool.bg, active_tool.bg), (TOOL_TAB_BG, TOOL_TAB_ACTIVE_BG));
        for bg in [TOOL_TAB_BG, TOOL_TAB_ACTIVE_BG] {
            assert!(bg != TAB_BG && bg != TOPBAR_BG && bg != ACCENT);
            let c = bg.to_srgba();
            assert!(c.blue > c.red && c.blue > c.green, "blue: {c:?}");
        }
        let asset = TabLook::of(false, false, false);
        assert_eq!((asset.icon, asset.bg, asset.ink), (true, TAB_BG, TEXT_DIM));
        assert_eq!(TabLook::of(false, true, false), TabLook { bg: TOPBAR_BG, ink: TEXT, icon: true });
    }

    /// Design §14.3: a greyed tab draws its text and icon in [`TAB_GREY`], whatever its kind: translucent (solid
    /// icons fade too) and, over a tab, dimmer than an inactive tab of the active domain.
    #[test]
    fn greyed_tabs_use_tab_grey() {
        for tool in [false, true] {
            assert_eq!(TabLook::of(tool, false, true).ink, TAB_GREY);
        }
        assert!(TAB_GREY.alpha() < 0.6);
        let (grey, bg) = (TAB_GREY.to_linear(), TAB_BG.to_linear());
        let over = |c: f32, b: f32| c * grey.alpha + b * (1.0 - grey.alpha);
        let shown = LinearRgba::rgb(over(grey.red, bg.red), over(grey.green, bg.green), over(grey.blue, bg.blue));
        assert!(Color::from(shown).luminance() < TEXT_DIM.luminance());
    }

    /// Design §14.3: the domain tabs' text is neutral, the active one's clearly brighter and set on the warm orange
    /// [`DOMAIN_TAB_ACTIVE_BG`] (apart from the tool tabs' blue), while the mainbar's [`MAINBAR_BG`] is darker than
    /// the other editor surfaces.
    #[test]
    fn domain_tabs_are_neutral_and_the_active_brighter() {
        use crate::theme::{
            DOMAIN_TAB_ACTIVE_BG, DOMAIN_TAB_ACTIVE_TEXT, DOMAIN_TAB_DISABLED_TEXT, DOMAIN_TAB_TEXT, MAINBAR_BG,
            PANEL_BG,
        };
        use crate::ui::shell::VIEW_BG;
        for c in [DOMAIN_TAB_TEXT, DOMAIN_TAB_ACTIVE_TEXT] {
            assert!(c != ACCENT && c != TOOL_TAB_BG && c != TOOL_TAB_ACTIVE_BG);
            let c = c.to_srgba();
            let spread = c.red.max(c.green).max(c.blue) - c.red.min(c.green).min(c.blue);
            assert!(spread < 0.08, "neutral: {c:?}");
        }
        assert!(DOMAIN_TAB_ACTIVE_TEXT.luminance() > DOMAIN_TAB_TEXT.luminance() * 1.5);
        assert_eq!(domain_tab_look(true, true), (DOMAIN_TAB_ACTIVE_BG, DOMAIN_TAB_ACTIVE_TEXT));
        assert_eq!(domain_tab_look(false, true), (Color::NONE, DOMAIN_TAB_TEXT));
        assert_eq!(domain_tab_look(false, false), (Color::NONE, DOMAIN_TAB_DISABLED_TEXT));
        assert_eq!(domain_tab_look(true, false), (Color::NONE, DOMAIN_TAB_DISABLED_TEXT));
        let active_bg = DOMAIN_TAB_ACTIVE_BG.to_srgba();
        assert!(active_bg.red > active_bg.green && active_bg.green > active_bg.blue, "orange: {active_bg:?}");
        assert!(DOMAIN_TAB_ACTIVE_TEXT.luminance() > DOMAIN_TAB_ACTIVE_BG.luminance() * 3.0);
        for surface in [VIEW_BG, TOPBAR_BG, PANEL_BG, HEADER_BG] {
            assert!(MAINBAR_BG.luminance() < surface.luminance(), "{surface:?}");
        }
    }

    /// Design §14.3: the secondbar's minimum is one tab row.
    #[test]
    fn tab_row_holds_one_row() {
        assert_eq!(TAB_ROW_MIN_H, ROW_PAD_TOP + TAB_H);
    }
}
