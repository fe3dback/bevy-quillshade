//! Modal dialogs (confirmations, name input, errors) and simple context menus.

use bevy::feathers::controls::{
    FeathersDialog, FeathersDialogBody, FeathersDialogFooter, FeathersDialogHeader, FeathersTextInput,
    FeathersTextInputContainer,
};
use bevy::feathers::display::caption;
use bevy::input::keyboard::KeyboardInput;
use bevy::input_focus::{AutoFocus, FocusedInput};
use bevy::prelude::*;
use bevy::text::{EditableText, TextEdit};
use bevy::ui_widgets::{Activate, Button, RequestClose};

use super::icons::{IconId, icon};
use super::tooltip::Tooltip;
use super::{HoverRow, OnActivate, TEXT, TEXT_DIM, button, text};
use crate::state::{Action, Actions, Dialog, Editor, Submit};

#[derive(Component, Default, Clone)]
pub struct DialogRoot;

/// The text field of an input dialog.
#[derive(Component, Default, Clone)]
pub struct DialogInput;

/// Text to put into a freshly spawned text field.
#[derive(Component, Default, Clone)]
pub struct InitText(pub String);

/// Submits the dialog's input text.
#[derive(Component, Default, Clone)]
pub struct SubmitButton;

#[derive(Component, Default, Clone)]
pub struct MenuRoot;

#[derive(Component, Default, Clone)]
pub struct MenuItem;

pub struct DialogPlugin;

impl Plugin for DialogPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (redraw, init_text))
            .add_observer(close_requested)
            .add_observer(submit_clicked)
            .add_observer(submit_on_enter)
            .add_observer(menu_item_activated)
            .add_observer(menu_barrier_clicked);
    }
}

fn redraw(
    editor: Res<Editor>,
    roots: Query<Entity, With<DialogRoot>>,
    mut last: Local<Option<Dialog>>,
    mut commands: Commands,
) {
    if *last == editor.dialog {
        return;
    }
    *last = editor.dialog.clone();
    for e in &roots {
        commands.entity(e).despawn();
    }
    let Some(d) = &editor.dialog else { return };
    let (title, mut body, buttons): (String, Vec<Box<dyn Scene>>, Vec<Box<dyn Scene>>) = match d {
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
            let mut body: Vec<Box<dyn Scene>> = vec![Box::new(text(label.clone(), TEXT_DIM, 13.0))];
            body.push(Box::new(bsn! {
                @FeathersTextInputContainer
                Node { min_width: px(380), flex_grow: 0.0 }
                Children [ @FeathersTextInput DialogInput init AutoFocus ]
            }));
            let ok = bsn! {
                @FeathersButton { @variant: ButtonVariant::Primary, @caption: bsn! { @caption("OK") } }
                SubmitButton
            };
            let mut buttons: Vec<Box<dyn Scene>> = Vec::new();
            if matches!(submit, Submit::OpenProject | Submit::NewProject) {
                buttons.push(Box::new(button("Browse...", Action::BrowseFolder, false)));
                buttons.push(Box::new(bsn! { Node { flex_grow: 1.0 } }));
            }
            buttons.push(Box::new(button("Cancel", Action::CloseDialog, false)));
            buttons.push(Box::new(ok));
            (title.clone(), body, buttons)
        }
        Dialog::About => {
            ("About".into(), vec![Box::new(about())], vec![Box::new(button("OK", Action::CloseDialog, true))])
        }
        Dialog::QuickStart(step) => {
            let last = super::welcome::QUICK_START_STEPS.len() - 1;
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
            ("Quick Start".into(), vec![Box::new(super::welcome::quick_start(*step))], b)
        }
    };
    if let Dialog::Input { submit: Submit::OpenProject, .. } = d {
        for p in editor.config.recent_projects.iter().take(6) {
            let path = p.display().to_string();
            body.push(Box::new(button(format!("Recent: {path}"), Action::OpenProject(p.clone()), false)));
        }
    }
    let width = px(match d {
        Dialog::About => 540,
        Dialog::QuickStart(_) => 820,
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

use bevy::feathers::controls::{ButtonVariant, FeathersButton};

/// Help → About: the logo, then name, version, link, author and license.
fn about() -> impl Scene {
    let version = format!("Version {}", crate::EDITOR_VERSION);
    let (author, license) = (format!("Author: {}", crate::AUTHORS), format!("License: {}", crate::LICENSE));
    let (act, hover) = (OnActivate(Action::OpenUrl(crate::PROJECT_URL.into())), HoverRow { base: Color::NONE });
    let tip = super::tooltip::Tooltip("Open in the browser".into());
    let link = crate::PROJECT_URL.trim_start_matches("https://");
    bsn! {
        Node { flex_direction: FlexDirection::Row, column_gap: px(18), align_items: AlignItems::Center }
        Children [
            @super::logo::logo(128.0)
            --
            Node { flex_direction: FlexDirection::Column, row_gap: px(6), min_width: px(0) }
            Children [
                @text("Bevy Shader Editor", TEXT, 20.0)
                --
                @text(version, TEXT_DIM, 13.0)
                --
                Node {
                    align_items: AlignItems::Center,
                    column_gap: px(6),
                    padding: UiRect::axes(px(4), px(2)),
                    margin: UiRect::left(px(-4)),
                    border_radius: BorderRadius::all(px(3)),
                }
                BackgroundColor(Color::NONE)
                Button
                act
                hover
                tip
                Children [
                    @icon(IconId::Link, 14.0, super::ACCENT)
                    --
                    @text(link, super::ACCENT, 13.0)
                ]
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

fn init_text(mut q: Query<(&InitText, &mut EditableText), Added<InitText>>) {
    for (init, mut t) in &mut q {
        t.queue_edit(TextEdit::SelectAll);
        t.queue_edit(TextEdit::Insert(init.0.as_str().into()));
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
    if ev.input.key_code == KeyCode::Enter
        && ev.input.state.is_pressed()
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

/// A menu item that draws a separator line.
pub fn menu_separator() -> (Option<IconId>, String, Action) {
    (None, String::new(), Action::None)
}

/// An item of a context menu.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuEntry {
    pub icon: Option<IconId>,
    /// `"Save\tCtrl+S"` shows its hotkey on the right; empty = a separator.
    pub label: String,
    pub action: Action,
    /// Shown dimmed and does nothing; the text (why) is its tooltip.
    pub disabled: Option<String>,
}

impl From<(Option<IconId>, String, Action)> for MenuEntry {
    fn from((icon, label, action): (Option<IconId>, String, Action)) -> Self {
        MenuEntry { icon, label, action, disabled: None }
    }
}

/// [`open_menu`] with an icon before each label; items without one keep the icon's space when any item has one.
/// A label `"Save\tCtrl+S"` shows its hotkey on the right; an empty label is a separator ([`menu_separator`]).
pub fn open_icon_menu(commands: &mut Commands, pos: Vec2, items: Vec<(Option<IconId>, String, Action)>) {
    open_menu_entries(commands, pos, items.into_iter().map(MenuEntry::from).collect());
}

/// [`open_icon_menu`] whose items may be disabled ([`MenuEntry::disabled`]).
pub fn open_menu_entries(commands: &mut Commands, pos: Vec2, items: Vec<MenuEntry>) {
    let with_icons = items.iter().any(|m| m.icon.is_some());
    let rows: Vec<Box<dyn Scene>> = items
        .into_iter()
        .map(|MenuEntry { icon: id, label, action, disabled }| -> Box<dyn Scene> {
            if label.is_empty() {
                return Box::new(bsn! {
                    Node { height: px(1), margin: UiRect::axes(px(6), px(4)) }
                    BackgroundColor(Color::srgb_u8(70, 72, 80))
                });
            }
            let (label, hotkey) = match label.split_once('\t') {
                Some((l, h)) => (l.to_string(), h.to_string()),
                None => (label, String::new()),
            };
            let color = if disabled.is_some() { TEXT_DIM } else { TEXT };
            let lead: Vec<Box<dyn Scene>> = match id {
                Some(id) => vec![Box::new(icon(id, 16.0, color))],
                None if with_icons => vec![Box::new(bsn! { Node { width: px(16), flex_shrink: 0.0 } })],
                None => Vec::new(),
            };
            match disabled {
                Some(why) => {
                    let tip = Tooltip(why);
                    Box::new(bsn! { @menu_row(lead, label, color, hotkey) tip })
                }
                None => {
                    let (act, hover) = (OnActivate(action), HoverRow { base: Color::NONE });
                    Box::new(bsn! { @menu_row(lead, label, color, hotkey) Button MenuItem act hover })
                }
            }
        })
        .collect();
    let (x, y) = (px(pos.x), px(pos.y));
    commands.spawn_scene(bsn! {
        MenuRoot
        Node { position_type: PositionType::Absolute, left: px(0), top: px(0), width: percent(100), height: percent(100) }
        GlobalZIndex(95)
        Children [
            Node {
                position_type: PositionType::Absolute,
                left: x,
                top: y,
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(px(4)),
                min_width: px(200),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(4)),
            }
            BackgroundColor(Color::srgb_u8(36, 38, 44))
            BorderColor::all(Color::srgb_u8(70, 72, 80))
            BoxShadow::new(Color::srgba(0.0, 0.0, 0.0, 0.6), px(0), px(3), px(0), px(10))
            Children [ {rows} ]
        ]
    });
}

/// A context menu row: icon (or its space), label, hotkey.
fn menu_row(lead: Vec<Box<dyn Scene>>, label: String, color: Color, hotkey: String) -> impl Scene {
    bsn! {
        Node {
            align_items: AlignItems::Center,
            column_gap: px(8),
            padding: UiRect::axes(px(10), px(4)),
            border_radius: BorderRadius::all(px(3)),
        }
        BackgroundColor(Color::NONE)
        Children [
            {lead}
            --
            @text(label, color, 13.0)
            --
            Node { flex_grow: 1.0, min_width: px(24) }
            --
            @text(hotkey, TEXT_DIM, 12.0)
        ]
    }
}

fn close_menus(commands: &mut Commands, menus: &Query<Entity, With<MenuRoot>>) {
    for m in menus {
        commands.entity(m).despawn();
    }
}

fn menu_item_activated(
    ev: On<Activate>,
    items: Query<(), With<MenuItem>>,
    menus: Query<Entity, With<MenuRoot>>,
    mut commands: Commands,
) {
    if items.contains(ev.event_target()) {
        close_menus(&mut commands, &menus);
    }
}

fn menu_barrier_clicked(ev: On<PointerPress>, menus: Query<Entity, With<MenuRoot>>, mut commands: Commands) {
    if menus.contains(ev.event_target()) && ev.original_event_target() == ev.event_target() {
        close_menus(&mut commands, &menus);
    }
}
