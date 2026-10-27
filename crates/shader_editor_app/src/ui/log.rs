//! Error/warning log (spec §9.5, 13): live validation + operation messages; click focuses the node. View → Show log
//! off (design §4) leaves only the pane's header bar: the counts, Clear and the show / hide button.

use bevy::feathers::rounded_corners::RoundedCorners;
use bevy::prelude::*;
use shader_graph_core::Diagnostic;

use super::icons::{IconId, UiIcon};
use super::widgets::{pane_box, pane_title};
use super::{BORDER, HoverRow, PANEL_BG, TEXT, TEXT_DIM, icon_button, plural, scroll_column, text, tool_button};
use crate::state::{Action, Actions, Editor, Level};

/// The console/logs container (under the editor statusbar).
#[derive(Component, Default, Clone)]
pub struct LogPane;

#[derive(Component, Default, Clone)]
pub struct LogBody;

/// The pane title: `Log - N errors, M warnings`.
#[derive(Component, Default, Clone)]
pub struct LogHeader;

/// The header's show / hide button; its chevron points where the log goes.
#[derive(Component, Default, Clone)]
pub struct LogToggle;

/// Height of the console/logs while the log is shown.
const LOG_HEIGHT: f32 = 190.0;

#[derive(Component, Default, Clone)]
pub struct LogRow(pub Option<Diagnostic>);

const ERR: Color = Color::srgb(0.95, 0.42, 0.42);
const WARN: Color = Color::srgb(0.95, 0.78, 0.35);
const ROW_LIMIT: usize = 300;

pub struct LogPlugin;

impl Plugin for LogPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (redraw, show_log)).add_observer(click_row);
    }
}

/// The console/logs: a pane, the counts in its title, Clear and the show / hide toggle in its header.
pub fn console() -> impl Scene {
    let toggle = icon_button(IconId::ChevronDown, Action::ToggleLog, "Show / hide log (`)");
    let tools: Vec<Box<dyn Scene>> =
        vec![Box::new(tool_button("Clear messages", Action::ClearMessages)), Box::new(bsn! { @{toggle} LogToggle })];
    bsn! {
        LogPane
        Node {
            height: px(LOG_HEIGHT),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(px(4)),
            border: UiRect::top(px(1)),
        }
        BackgroundColor(PANEL_BG)
        BorderColor::all(BORDER)
        Children [
            @pane_box(bsn! { @pane_title("Log") LogHeader }, tools, bsn! { @scroll_column() LogBody })
            Node { flex_grow: 1.0, min_height: px(0) }
        ]
    }
}

/// Follows [`UserConfig::show_log`](crate::config::UserConfig::show_log): hidden, the pane body is not displayed, the
/// container takes the header's height and the header gets the pane's bottom corners and border. Runs when the flag
/// changed or the console was spawned.
#[allow(clippy::too_many_arguments)]
fn show_log(
    editor: Res<Editor>,
    mut last: Local<Option<bool>>,
    spawned: Query<(), Added<LogPane>>,
    mut panes: Query<&mut Node, With<LogPane>>,
    bodies: Query<&ChildOf, With<LogBody>>,
    headers: Query<&ChildOf, With<LogHeader>>,
    mut nodes: Query<&mut Node, Without<LogPane>>,
    toggles: Query<Entity, With<LogToggle>>,
    children: Query<&Children>,
    mut icons: Query<&mut UiIcon>,
) {
    let shown = editor.config.show_log;
    if *last == Some(shown) && spawned.is_empty() {
        return;
    }
    *last = Some(shown);
    let height = if shown { px(LOG_HEIGHT) } else { Val::Auto };
    for mut node in &mut panes {
        if node.height != height {
            node.height = height;
        }
    }
    let display = if shown { Display::Flex } else { Display::None };
    for body in &bodies {
        if let Ok(mut node) = nodes.get_mut(body.parent())
            && node.display != display
        {
            node.display = display;
        }
    }
    let corners = if shown { RoundedCorners::Top } else { RoundedCorners::All };
    let bottom = if shown { px(0) } else { px(1) };
    for header in &headers {
        if let Ok(mut node) = nodes.get_mut(header.parent()) {
            node.border_radius = corners.to_border_radius(4.0);
            node.border.bottom = bottom;
        }
    }
    let chevron = if shown { IconId::ChevronDown } else { IconId::ChevronUp };
    for toggle in &toggles {
        for e in children.iter_descendants(toggle) {
            if let Ok(mut icon) = icons.get_mut(e)
                && icon.id != chevron
            {
                icon.id = chevron;
            }
        }
    }
}

fn row(label: String, color: Color, diag: Option<Diagnostic>) -> impl Scene {
    let (r, hover) = (LogRow(diag), HoverRow { base: Color::NONE });
    bsn! {
        Node { padding: UiRect::axes(px(6), px(1)), border_radius: BorderRadius::all(px(2)) }
        BackgroundColor(Color::NONE)
        r
        hover
        Children [ @text(label, color, 12.0) ]
    }
}

fn redraw(
    editor: Res<Editor>,
    body: Query<Entity, With<LogBody>>,
    mut header: Query<&mut Text, With<LogHeader>>,
    mut last: Local<Option<u64>>,
    mut scroll: Query<&mut ScrollPosition, With<LogBody>>,
    mut commands: Commands,
) {
    if *last == Some(editor.ui.log) {
        return;
    }
    let Ok(body) = body.single() else { return };
    *last = Some(editor.ui.log);
    commands.entity(body).despawn_children();
    let diags = editor.all_diagnostics();
    let (errors, warnings) =
        (diags.iter().filter(|d| d.is_error()).count(), diags.iter().filter(|d| !d.is_error()).count());
    for mut h in &mut header {
        h.0 = format!("Log - {}, {}", plural(errors, "error", "errors"), plural(warnings, "warning", "warnings"));
    }
    for d in diags.iter().take(ROW_LIMIT) {
        let color = if d.is_error() { ERR } else { WARN };
        commands.spawn_scene(bsn! { @row(d.to_string(), color, Some(d.clone())) ChildOf(body) });
    }
    if diags.is_empty() && editor.project.is_some() {
        commands.spawn_scene(bsn! { @row("No problems.".into(), TEXT_DIM, None) ChildOf(body) });
    }
    for m in editor.messages.iter().rev().take(50).rev() {
        let color = match m.level {
            Level::Info => TEXT,
            Level::Warning => WARN,
            Level::Error => ERR,
        };
        commands.spawn_scene(bsn! { @row(format!("> {}", m.text), color, None) ChildOf(body) });
    }
    if let Ok(mut s) = scroll.single_mut() {
        s.y = f32::MAX;
    }
}

fn click_row(ev: On<PointerClick>, rows: Query<&LogRow>, mut actions: ResMut<Actions>) {
    if let Ok(LogRow(Some(d))) = rows.get(ev.event_target()) {
        actions.push(Action::FocusDiagnostic(d.clone()));
    }
}
