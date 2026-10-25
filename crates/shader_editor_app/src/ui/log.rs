//! Error/warning log (spec §9.5, 13): live validation + operation messages; click focuses the node.

use bevy::prelude::*;
use shader_graph_core::Diagnostic;

use super::{HoverRow, TEXT, TEXT_DIM, text};
use crate::state::{Action, Actions, Editor, Level};

#[derive(Component, Default, Clone)]
pub struct LogBody;

#[derive(Component, Default, Clone)]
pub struct LogHeader;

#[derive(Component, Default, Clone)]
pub struct LogRow(pub Option<Diagnostic>);

const ERR: Color = Color::srgb(0.95, 0.42, 0.42);
const WARN: Color = Color::srgb(0.95, 0.78, 0.35);
const ROW_LIMIT: usize = 300;

pub struct LogPlugin;

impl Plugin for LogPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, redraw).add_observer(click_row);
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
        h.0 = format!("Log - {errors} error(s), {warnings} warning(s)");
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
