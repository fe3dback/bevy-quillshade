//! Error/warning log (spec §9.5, 13, design §6.4): every domain's diagnostics ([`Editor::console_diagnostics`]) and
//! the operation messages; click focuses the node. View → Show console off (design §3) leaves only the pane's header
//! bar: the counts, Clear and the show / hide button; Zen mode hides the whole console. Info diagnostics (spec §13)
//! are listed, dimmed after the errors and warnings, only while the header's "Info" checkbox is on; the title counts
//! them separately.

use crate::action::{Action, Actions};
use crate::config::Panel;
use crate::message::Level;
use crate::theme::{BORDER, PANEL_BG, TEXT, TEXT_DIM, text};
use crate::ui::HoverRow;
use crate::ui::icons::{IconId, IconRef, UiIcon};
use crate::ui::widgets::{pane_box, pane_title, scroll_column};
use crate::util::text::plural;
use bevy::feathers::controls::FeathersCheckbox;
use bevy::feathers::display::caption;
use bevy::feathers::rounded_corners::RoundedCorners;
use bevy::prelude::*;
use bevy::ui::Checked;
use bevy::ui_widgets::ValueChange;
use project_core::{Diagnostic, DomainId, Level as DiagLevel};

use super::{areas, icon_button, tool_button};
use crate::state::editor::Editor;

/// The console/logs container (under the editor statusbar).
#[derive(Component, Default, Clone)]
pub struct LogPane;

#[derive(Component, Default, Clone)]
pub struct LogBody;

/// The pane title: `Log - N errors, M warnings` (`, K info` while there are info diagnostics).
#[derive(Component, Default, Clone)]
pub struct LogHeader;

/// The header's show / hide button; its chevron points where the log goes.
#[derive(Component, Default, Clone)]
pub struct LogToggle;

/// The header's "All domains" checkbox (design §6.4).
#[derive(Component, Default, Clone)]
pub struct ConsoleAllToggle;

/// The header's "Info" checkbox (spec §13): lists the info diagnostics too.
#[derive(Component, Default, Clone)]
pub struct ConsoleInfoToggle;

/// Height of the console/logs while the log is shown.
const LOG_HEIGHT: f32 = 190.0;

#[derive(Component, Default, Clone)]
pub struct LogRow(pub Option<Diagnostic>);

/// Error lines (also the Migration Tool's, [`super::migration`]).
pub(crate) const ERR: Color = Color::srgb(0.95, 0.42, 0.42);
/// Warning lines (also the Migration Tool's, [`super::migration`]).
pub(crate) const WARN: Color = Color::srgb(0.95, 0.78, 0.35);
const ROW_LIMIT: usize = 300;

pub struct LogPlugin;

impl Plugin for LogPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (redraw, show_log).after(crate::apply_actions))
            .add_observer(click_row)
            .add_observer(toggle_all)
            .add_observer(toggle_info);
    }
}

/// The console/logs: a pane, the counts in its title, Clear and the show / hide toggle in its header.
pub fn console() -> impl Scene {
    let toggle = icon_button(IconId::ChevronDown, Action::ToggleLog, "Show / hide console (Ctrl+2)");
    let all = bsn! {
        @FeathersCheckbox { @caption: bsn! { @caption("All domains") } }
        Node { flex_shrink: 0.0 }
        ConsoleAllToggle
    };
    let info = bsn! {
        @FeathersCheckbox { @caption: bsn! { @caption("Info") } }
        Node { flex_shrink: 0.0 }
        ConsoleInfoToggle
    };
    let tools: Vec<Box<dyn Scene>> = vec![
        Box::new(all),
        Box::new(info),
        Box::new(tool_button("Clear messages", Action::ClearMessages)),
        Box::new(bsn! { @{toggle} LogToggle }),
    ];
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

/// Follows [`Panel::Log`]: hidden, the pane body is not displayed, the container takes the header's height and the
/// header gets the pane's bottom corners and border. Zen mode ([`Panel::Console`]) hides the whole console, header
/// included, out of Tab's reach too ([`areas::display`]). Runs when either changed or the console was spawned.
#[allow(clippy::too_many_arguments)]
fn show_log(
    editor: Res<Editor>,
    mut last: Local<Option<(bool, bool)>>,
    spawned: Query<(), Added<LogPane>>,
    mut panes: Query<(&mut Node, &mut Visibility), With<LogPane>>,
    bodies: Query<&ChildOf, With<LogBody>>,
    headers: Query<&ChildOf, With<LogHeader>>,
    mut nodes: Query<&mut Node, Without<LogPane>>,
    toggles: Query<Entity, With<LogToggle>>,
    children: Query<&Children>,
    mut icons: Query<&mut UiIcon>,
) {
    let (console, shown) = (editor.config.panel_visible(Panel::Console), editor.config.panel_visible(Panel::Log));
    if *last == Some((console, shown)) && spawned.is_empty() {
        return;
    }
    *last = Some((console, shown));
    let height = if shown { px(LOG_HEIGHT) } else { Val::Auto };
    for (mut node, mut visibility) in &mut panes {
        if node.height != height {
            node.height = height;
        }
        areas::display(&mut node, &mut visibility, console);
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
    let chevron: IconRef = if shown { IconId::ChevronDown } else { IconId::ChevronUp }.into();
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

/// The pane title for `diags`: errors and warnings, then `, K info` while there are info diagnostics (listed or not).
pub(crate) fn header_text(diags: &[Diagnostic]) -> String {
    let count = |level: DiagLevel| diags.iter().filter(|d| d.level == level).count();
    let mut out = format!(
        "Log - {}, {}",
        plural(count(DiagLevel::Error), "error", "errors"),
        plural(count(DiagLevel::Warning), "warning", "warnings")
    );
    let info = count(DiagLevel::Info);
    if info > 0 {
        out.push_str(&format!(", {info} info"));
    }
    out
}

/// The listed diagnostics: errors and warnings in their console order, then the infos while `info` is on.
pub(crate) fn listed(diags: &[Diagnostic], info: bool) -> (Vec<&Diagnostic>, Vec<&Diagnostic>) {
    let problems = diags.iter().filter(|d| !d.is_info()).collect();
    let infos = if info { diags.iter().filter(|d| d.is_info()).collect() } else { Vec::new() };
    (problems, infos)
}

fn redraw(
    editor: Res<Editor>,
    body: Query<Entity, With<LogBody>>,
    mut header: Query<&mut Text, With<LogHeader>>,
    mut last: Local<Option<(u64, Option<DomainId>, bool, bool)>>,
    mut scroll: Query<&mut ScrollPosition, With<LogBody>>,
    mut commands: Commands,
) {
    let key = (editor.ui.log, editor.active_domain, editor.console_all, editor.console_info);
    if *last == Some(key) {
        return;
    }
    let Ok(body) = body.single() else { return };
    *last = Some(key);
    commands.entity(body).despawn_children();
    let diags = editor.console_diagnostics();
    for mut h in &mut header {
        h.0 = header_text(&diags);
    }
    let (problems, infos) = listed(&diags, editor.console_info);
    for d in problems.iter().take(ROW_LIMIT) {
        let color = if d.is_error() { ERR } else { WARN };
        commands.spawn_scene(bsn! { @row(d.to_string(), color, Some((*d).clone())) ChildOf(body) });
    }
    if problems.is_empty() && editor.project.is_some() {
        commands.spawn_scene(bsn! { @row("No problems.".into(), TEXT_DIM, None) ChildOf(body) });
    }
    for d in infos.iter().take(ROW_LIMIT.saturating_sub(problems.len())) {
        commands.spawn_scene(bsn! { @row(d.to_string(), TEXT_DIM, Some((*d).clone())) ChildOf(body) });
    }
    let messages: Vec<_> = editor.console_messages().collect();
    for m in messages.iter().rev().take(50).rev() {
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

/// The "All domains" checkbox: Feathers leaves its checked state to the app, so the model's flag is the source.
fn toggle_all(
    ev: On<ValueChange<bool>>,
    boxes: Query<(), With<ConsoleAllToggle>>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    if boxes.contains(ev.source) {
        if ev.value {
            commands.entity(ev.source).insert(Checked);
        } else {
            commands.entity(ev.source).remove::<Checked>();
        }
        actions.push(Action::ToggleConsoleAll);
    }
}

/// The "Info" checkbox: like "All domains", the model's flag is the source.
fn toggle_info(
    ev: On<ValueChange<bool>>,
    boxes: Query<(), With<ConsoleInfoToggle>>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    if boxes.contains(ev.source) {
        if ev.value {
            commands.entity(ev.source).insert(Checked);
        } else {
            commands.entity(ev.source).remove::<Checked>();
        }
        actions.push(Action::ToggleConsoleInfo);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diags() -> Vec<Diagnostic> {
        vec![
            Diagnostic::info("I103", "uv1"),
            Diagnostic::error("E003", "bad"),
            Diagnostic::warning("W104", "truncated"),
            Diagnostic::info("I103", "color"),
        ]
    }

    #[test]
    fn header_counts_info_apart() {
        assert_eq!(header_text(&diags()), "Log - 1 error, 1 warning, 2 info");
        assert_eq!(header_text(&diags()[1..3]), "Log - 1 error, 1 warning");
        assert_eq!(header_text(&[]), "Log - 0 errors, 0 warnings");
    }

    #[test]
    fn infos_listed_last_and_only_when_asked() {
        let all = diags();
        let codes = |v: &[&Diagnostic]| v.iter().map(|d| d.code.clone()).collect::<Vec<_>>();
        let (problems, infos) = listed(&all, false);
        assert_eq!((codes(&problems), infos.len()), (vec!["E003".to_string(), "W104".into()], 0));
        let (problems, infos) = listed(&all, true);
        assert_eq!(problems.len(), 2);
        assert_eq!(infos.iter().map(|d| d.message.as_str()).collect::<Vec<_>>(), ["uv1", "color"]);
    }
}
