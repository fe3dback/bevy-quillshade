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

use super::{HoverRow, OnActivate, TEXT, TEXT_DIM, button, text};
use crate::state::{Action, Actions, Dialog, Editor};

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
        Dialog::Input { title, label, initial, .. } => {
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
            (title.clone(), body, vec![Box::new(button("Cancel", Action::CloseDialog, false)), Box::new(ok)])
        }
    };
    if let Dialog::Input { submit: crate::state::Submit::OpenProject, .. } = d {
        for p in editor.config.recent.iter().take(6) {
            let path = p.display().to_string();
            body.push(Box::new(button(format!("Recent: {path}"), Action::OpenProject(p.clone()), false)));
        }
    }
    commands.spawn_scene(bsn! {
        @FeathersDialog {
            @width: px(460),
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
    let rows: Vec<Box<dyn Scene>> = items
        .into_iter()
        .map(|(label, action)| -> Box<dyn Scene> {
            let (act, hover) = (OnActivate(action), HoverRow { base: Color::NONE });
            Box::new(bsn! {
                Node { padding: UiRect::axes(px(10), px(4)), border_radius: BorderRadius::all(px(3)) }
                BackgroundColor(Color::NONE)
                Button
                MenuItem
                act
                hover
                Children [ @text(label, TEXT, 13.0) ]
            })
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
