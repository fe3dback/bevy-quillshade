//! Modal dialogs (confirmations, name input, errors) and the menus: context menus at the cursor and the popups of
//! the main menu, both Feathers menus ([`menu_popup`]).

use bevy::feathers::controls::{
    ButtonVariant, FeathersButton, FeathersDialog, FeathersDialogBody, FeathersDialogFooter, FeathersDialogHeader,
    FeathersMenuDivider, FeathersMenuItem, FeathersMenuPopup, FeathersTextInput, FeathersTextInputContainer,
};
use bevy::feathers::display::caption;
use bevy::input::keyboard::KeyboardInput;
use bevy::input_focus::tab_navigation::NavAction;
use bevy::input_focus::{AcquireFocus, AutoFocus, FocusedInput, InputFocus};
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::{Activate, MenuAction, MenuEvent, MenuFocusState, RequestClose};

use crate::action::{Action, Actions, Dialog, Submit};
use crate::logo;
use crate::theme::{ACCENT, BORDER, TEXT, TEXT_DIM, text};
use crate::tooltip::Tooltip;
use crate::ui::icons::{IconId, IconRef, icon};

use super::{OnActivate, button};
use crate::domain::DomainList;
use crate::state::editor::Editor;

#[derive(Component, Default, Clone)]
pub struct DialogRoot;

/// The text field of an input dialog.
#[derive(Component, Default, Clone)]
pub struct DialogInput;

/// Text to put into a freshly spawned text field.
pub use crate::ui::props::InitText;

/// Submits the dialog's input text.
#[derive(Component, Default, Clone)]
pub struct SubmitButton;

/// An open context menu ([`menu_barrier`], [`open_menu_entries`]).
pub use super::MenuRoot;

pub struct DialogPlugin;

impl Plugin for DialogPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, redraw.after(crate::apply_actions))
            .add_observer(close_requested)
            .add_observer(submit_clicked)
            .add_observer(submit_on_enter);
    }
}

pub(super) fn redraw(
    editor: Res<Editor>,
    list: Res<DomainList>,
    roots: Query<Entity, With<DialogRoot>>,
    mut last: Local<Option<Dialog>>,
    mut commands: Commands,
) {
    if *last == editor.dialog {
        return;
    }
    *last = editor.dialog.clone();
    for e in &roots {
        commands.entity(e).try_despawn();
    }
    let Some(d) = &editor.dialog else { return };
    let (title, body, buttons): (String, Vec<Box<dyn Scene>>, Vec<Box<dyn Scene>>) = match d {
        Dialog::Message { title, message } => (
            title.clone(),
            vec![Box::new(text(message.clone(), TEXT, 14.0))],
            vec![Box::new(button("OK", Action::CloseDialog, true))],
        ),
        Dialog::Confirm { title, message, buttons } => {
            let n = buttons.len();
            let b = buttons
                .iter()
                .enumerate()
                .map(|(i, (l, a))| -> Box<dyn Scene> { Box::new(button(l.clone(), a.clone(), i + 1 == n)) })
                .collect();
            (title.clone(), vec![Box::new(text(message.clone(), TEXT, 14.0))], b)
        }
        Dialog::Input { title, label, initial, submit } => {
            let init = InitText(initial.clone());
            let max = match submit {
                Submit::SetText(target) => target.max_chars(),
                _ => None,
            };
            let mut body: Vec<Box<dyn Scene>> = vec![Box::new(text(label.clone(), TEXT_DIM, 13.0))];
            let field = bsn! {
                @FeathersTextInputContainer
                Node { min_width: px(0), flex_grow: 1.0 }
                Children [ @FeathersTextInput EditableText { max_characters: {max} } DialogInput init AutoFocus ]
            };
            // A directory: Browse... (the native folder picker) next to its field.
            if matches!(submit, Submit::OpenProject | Submit::NewProject) {
                body.push(Box::new(bsn! {
                    Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, column_gap: px(8) }
                    Children [ @{field} -- @button("Browse...", Action::BrowseFolder, false) ]
                }));
            } else {
                body.push(Box::new(bsn! { Node { min_width: px(380) } Children [ @{field} ] }));
            }
            if let Submit::OpenProject = submit {
                body.push(Box::new(recent_projects()));
            }
            let ok_label = if matches!(submit, Submit::OpenProject) { "Open" } else { "OK" };
            let ok = bsn! {
                @FeathersButton { @variant: ButtonVariant::Primary, @caption: bsn! { @caption(ok_label) } }
                SubmitButton
            };
            let buttons: Vec<Box<dyn Scene>> =
                vec![Box::new(button("Cancel", Action::CloseDialog, false)), Box::new(ok)];
            (title.clone(), body, buttons)
        }
        Dialog::About => {
            ("About".into(), vec![Box::new(about())], vec![Box::new(button("OK", Action::CloseDialog, true))])
        }
        Dialog::QuickStart(step) => {
            let steps = crate::quick_start::steps(&list);
            let last = steps.len().saturating_sub(1);
            let mut b: Vec<Box<dyn Scene>> = Vec::new();
            if *step > 0 {
                b.push(Box::new(button("Back", Action::QuickStart(step - 1), false)));
            }
            b.push(Box::new(bsn! { Node { flex_grow: 1.0 } }));
            if *step < last {
                b.push(Box::new(button("Close", Action::CloseDialog, false)));
                b.push(Box::new(button("Next", Action::QuickStart(step + 1), true)));
            } else {
                b.push(Box::new(button("Done", Action::CloseDialog, true)));
            }
            let body: Vec<Box<dyn Scene>> = vec![Box::new(super::welcome::quick_start(steps, *step))];
            ("Quick Start".into(), body, b)
        }
    };
    let width = px(match d {
        Dialog::About => 540,
        Dialog::QuickStart(_) => 820,
        Dialog::Input { submit: Submit::OpenProject | Submit::NewProject, .. } => 540,
        _ => 460,
    });
    commands.spawn_scene(bsn! {
        @FeathersDialog {
            @width: width,
            @contents: bsn_list! {
                @FeathersDialogHeader Children [ @caption(title) ]
                --
                @FeathersDialogBody Node { flex_direction: FlexDirection::Column, row_gap: px(8) } Children [ {body} ]
                --
                @FeathersDialogFooter Children [ {buttons} ]
            }
        }
        DialogRoot
    });
}

/// How many recent projects the Open project dialog lists.
const DIALOG_RECENT: usize = 5;

/// The Open project dialog's recent projects, apart from the field above them: a separator line, a heading and
/// the [`RecentList`](super::welcome::RecentList) of the last [`DIALOG_RECENT`], by name.
fn recent_projects() -> impl Scene {
    bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(4),
            margin: UiRect::top(px(10)),
            padding: UiRect::top(px(12)),
            border: UiRect::top(px(1)),
        }
        BorderColor::all(BORDER)
        Children [
            @text("Recent projects", TEXT, 14.0)
            --
            super::welcome::RecentList { limit: DIALOG_RECENT, compact: true }
            Node { flex_direction: FlexDirection::Column, row_gap: px(1) }
        ]
    }
}

/// Help → About: the logo, then name, version, link, author and license.
fn about() -> impl Scene {
    let version = format!("Version {}", crate::EDITOR_VERSION);
    let (author, license) = (format!("Author: {}", crate::AUTHORS), format!("License: {}", crate::LICENSE));
    let act = OnActivate(Action::OpenUrl(crate::PROJECT_URL.into()));
    let tip = Tooltip("Open in the browser".into());
    let link = crate::PROJECT_URL.trim_start_matches("https://");
    bsn! {
        Node { flex_direction: FlexDirection::Row, column_gap: px(18), align_items: AlignItems::Center }
        Children [
            @logo::logo(128.0)
            --
            Node { flex_direction: FlexDirection::Column, row_gap: px(6), min_width: px(0) }
            Children [
                @text(crate::EDITOR_TITLE, TEXT, 20.0)
                --
                @text(version, TEXT_DIM, 13.0)
                --
                @FeathersButton {
                    @variant: ButtonVariant::Plain,
                    @caption: bsn_list! { @icon(IconId::Link, 14.0, ACCENT) -- @text(link, ACCENT, 13.0) },
                }
                Node {
                    align_self: AlignSelf::Start,
                    column_gap: px(6),
                    padding: UiRect::axes(px(4), px(2)),
                    margin: UiRect::left(px(-4)),
                    height: Val::Auto,
                }
                act
                tip
                --
                @text(author, TEXT, 13.0)
                --
                @text(license, TEXT, 13.0)
                --
                @text(crate::DISCLAIMER, TEXT_DIM, 11.0) Node { margin: UiRect::top(px(6)) }
            ]
        ]
    }
}

fn close_requested(
    ev: On<RequestClose>,
    roots: Query<(), With<DialogRoot>>,
    parents: Query<&ChildOf>,
    mut actions: ResMut<Actions>,
) {
    let target = ev.event_target();
    let in_dialog = roots.contains(target) || parents.iter_ancestors(target).any(|a| roots.contains(a));
    if in_dialog {
        actions.push(Action::CloseDialog);
    }
}

fn submit_text(inputs: &Query<&EditableText, With<DialogInput>>) -> Option<String> {
    inputs.iter().next().map(|t| t.value().to_string())
}

fn submit_clicked(
    ev: On<Activate>,
    submit: Query<(), With<SubmitButton>>,
    inputs: Query<&EditableText, With<DialogInput>>,
    mut actions: ResMut<Actions>,
) {
    if submit.contains(ev.event_target())
        && let Some(t) = submit_text(&inputs)
    {
        actions.push(Action::Submit(t));
    }
}

fn submit_on_enter(
    ev: On<FocusedInput<KeyboardInput>>,
    inputs: Query<&EditableText, With<DialogInput>>,
    mut actions: ResMut<Actions>,
) {
    // A held Enter's key repeat never submits: the dialog may open while the Enter that caused it is still down
    // (a Named Route picked in the palette, design §6.4).
    if ev.input.key_code == KeyCode::Enter
        && ev.input.state.is_pressed()
        && !ev.input.repeat
        && inputs.contains(ev.focused_entity)
        && let Some(t) = submit_text(&inputs)
    {
        actions.push(Action::Submit(t));
    }
}

/// Opens a context menu at `pos` (window logical px).
pub fn open_menu(commands: &mut Commands, pos: Vec2, items: Vec<(String, Action)>) {
    open_icon_menu(commands, pos, items.into_iter().map(|(label, action)| (None, label, action)).collect());
}

pub use crate::menu::{MenuEntry, menu_separator};

/// [`open_menu`] with an icon before each label; items without one keep the icon's space when any item has one.
/// A label `"Save\tCtrl+S"` shows its hotkey on the right; an empty label is a separator ([`menu_separator`]).
pub fn open_icon_menu(commands: &mut Commands, pos: Vec2, items: Vec<(Option<IconRef>, String, Action)>) {
    open_menu_entries(commands, pos, items.into_iter().map(MenuEntry::from).collect());
}

/// [`open_icon_menu`] whose items may be disabled ([`MenuEntry::disabled`]).
///
/// A Feathers menu popup ([`menu_popup`]) on a [`menu_barrier`] ([`MenuRoot`]), hung from a zero-size anchor at
/// `pos`: it opens below the point (above it when there is no room), focuses its first item and closes when the focus
/// leaves it, on Esc or when an item is activated; the barrier goes with it ([`context_menu_event`]). A click outside
/// lands on the barrier: it only closes the menu. One context menu at a time: opening one removes any other.
pub fn open_menu_entries(commands: &mut Commands, pos: Vec2, items: Vec<MenuEntry>) {
    commands.queue(|world: &mut World| {
        let open: Vec<Entity> = world.query_filtered::<Entity, With<MenuRoot>>().iter(world).collect();
        for e in open {
            world.entity_mut(e).despawn();
        }
    });
    let popup = menu_popup(items);
    let (x, y) = (px(pos.x), px(pos.y));
    commands.spawn_scene(bsn! {
        @menu_barrier()
        MenuRoot
        on(context_menu_event)
        Children [
            Node { position_type: PositionType::Absolute, left: x, top: y, width: px(0), height: px(0) }
            Children [ @{popup} Visibility::Visible MenuFocusState::Opening(NavAction::First) ]
        ]
    });
}

/// A transparent node over the whole window, under an open menu's popup and over everything else: a press outside
/// the popup lands here and nowhere else. It takes no focus, so its `AcquireFocus` reaches the window, which clears
/// the focus; the menu then closes on its focus loss (Feathers).
pub fn menu_barrier() -> impl Scene {
    bsn! {
        Node { position_type: PositionType::Absolute, left: px(0), top: px(0), width: percent(100), height: percent(100) }
        GlobalZIndex(MENU_BARRIER_Z)
    }
}

/// Z of a [`menu_barrier`]; menu popups are one above it, tooltips above both.
pub const MENU_BARRIER_Z: i32 = 94;

/// The popup of a main or context menu: a `FeathersMenuPopup` of [`menu_rows`], at least 200 px wide, above its
/// [`menu_barrier`] and under the tooltips (which name why an item is disabled). A press inside it that hits no item
/// (the padding, a divider) keeps the focus where it is, so it does not close the menu.
pub fn menu_popup(items: Vec<MenuEntry>) -> impl Scene {
    let rows = menu_rows(items);
    bsn! {
        @FeathersMenuPopup
        Node { min_width: px(200) }
        GlobalZIndex({ MENU_BARRIER_Z + 1 })
        on(|mut ev: On<AcquireFocus>| ev.propagate(false))
        Children [ {rows} ]
    }
}

/// The rows of a menu popup: a `FeathersMenuDivider` per separator, a `FeathersMenuItem` per item — icon (or its
/// space when any item has one), label (a Feathers caption), the hotkey dimmed on the right. An activated item pushes its action
/// ([`OnActivate`]); a disabled one is `InteractionDisabled` (dimmed, no `Activate`) with its reason as tooltip.
pub fn menu_rows(items: Vec<MenuEntry>) -> Vec<Box<dyn Scene>> {
    let with_icons = items.iter().any(|m| m.icon.is_some());
    items
        .into_iter()
        .map(|MenuEntry { icon: id, label, action, disabled }| -> Box<dyn Scene> {
            if label.is_empty() {
                return Box::new(bsn! { @FeathersMenuDivider });
            }
            let (label, hotkey) = match label.split_once('\t') {
                Some((l, h)) => (l.to_string(), h.to_string()),
                None => (label, String::new()),
            };
            let tint = if disabled.is_some() { TEXT_DIM } else { TEXT };
            let lead: Vec<Box<dyn Scene>> = match id {
                Some(id) => vec![Box::new(icon(id, 16.0, tint))],
                None if with_icons => vec![Box::new(bsn! { Node { width: px(16), flex_shrink: 0.0 } })],
                None => Vec::new(),
            };
            // The label is a Feathers caption: the item's theme colour (dimmed when disabled) and font reach it.
            let caption = bsn_list! {
                {lead}
                --
                @caption(label)
                --
                Node { flex_grow: 1.0, min_width: px(24) }
                --
                @text(hotkey, TEXT_DIM, 12.0)
            };
            match disabled {
                Some(why) => {
                    let tip = Tooltip(why);
                    Box::new(bsn! {
                        @FeathersMenuItem { @caption: caption } Node { column_gap: px(8) } InteractionDisabled tip
                    })
                }
                None => {
                    let act = OnActivate(action);
                    Box::new(bsn! { @FeathersMenuItem { @caption: caption } Node { column_gap: px(8) } act })
                }
            }
        })
        .collect()
}

/// Closes a context menu: on `CloseAll` (focus lost, Esc, an item activated) its [`MenuRoot`] goes; `FocusRoot` (Esc,
/// an item activated) drops the keyboard focus, as a context menu has no button to return it to.
fn context_menu_event(
    mut ev: On<MenuEvent>,
    focus: Option<ResMut<InputFocus>>,
    parents: Query<&ChildOf>,
    mut commands: Commands,
) {
    let root = ev.event_target();
    match ev.action {
        MenuAction::CloseAll => {
            ev.propagate(false);
            commands.entity(root).try_despawn();
        }
        MenuAction::FocusRoot => {
            ev.propagate(false);
            if let Some(mut focus) = focus
                && focus.get().is_some_and(|f| f == root || parents.iter_ancestors(f).any(|a| a == root))
            {
                focus.clear();
            }
        }
        MenuAction::Open(_) | MenuAction::Toggle => {}
    }
}
