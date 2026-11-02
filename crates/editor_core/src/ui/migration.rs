//! The Migration Tool modal (ADR 0046 "Editor (Migration Tool modal)", decision D28, techdoc §4.4 "UI"): the view
//! of [`Editor::migration`] ([`crate::state::migration`]), a Feathers modal dialog over the whole window while a
//! [`MigrationSession`] exists. The rest of the editor is unavailable meanwhile: the dialog's barrier takes every
//! click, [`Editor::apply`] drops every other action and [`crate::main_input::Typing`] holds back the hotkeys.
//!
//! It shows the project path, the version range (`v1 -> v2`), the git state ("commit first"), the orchestrator's
//! warnings, the steps with their titles and stale files grouped by domain, a progress bar while the migration runs
//! and after it, the session log (newest at the bottom) and, by phase ([`footer_buttons`]): Cancel / Dry run /
//! **Migrate** when ready, everything disabled while running, **Open project** when done, Cancel / Dry run /
//! **Retry** after a failure. A click outside the dialog or Esc does nothing: leaving it is an explicit Cancel.
//! Its texts are ASCII (`->`, `:`) like the CLI's: the default UI font has no arrows or dashes.
//!
//! [`redraw`] despawns and respawns the modal when the session moves (its root, `rev` or phase), the same pattern
//! as [`super::dialog`].
//!
//! Keyboard focus: Feathers' modal focuses its last focusable child shortly after it spawns unless the focus is
//! already inside, and Enter / Space press a focused Feathers button past the editor's hotkey gate. The last button
//! is **Migrate** / **Retry**, and the modal respawns after every Dry run, so [`FooterButton::autofocus`] puts the
//! focus on **Cancel** instead (`AutoFocus`) whenever it is pressable: a stray Enter never starts the migration.

use std::path::PathBuf;

use bevy::feathers::controls::{
    ButtonVariant, FeathersButton, FeathersDialog, FeathersDialogBody, FeathersDialogFooter, FeathersDialogHeader,
};
use bevy::feathers::display::caption;
use bevy::input_focus::AutoFocus;
use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use project_migrate::GitState;

use crate::action::MigrationAction;
use crate::state::editor::Editor;
use crate::state::migration::{MigrationPhase, MigrationSession};
use crate::theme::{ACCENT, BORDER, TEXT, TEXT_DIM, text};
use crate::tooltip::Tooltip;
use crate::ui::icons::{IconId, icon};
use crate::util::text::plural;

use super::OnActivate;
use super::log::{ERR, WARN};
use super::shell::{VIEW_BG, clipped_text};
use super::widgets::scroll_column;

/// The Migration Tool modal's root (the Feathers dialog with its full-window barrier).
#[derive(Component, Default, Clone)]
pub struct MigrationRoot;

/// The scrollable session log inside the modal (kept scrolled to its newest line).
#[derive(Component, Default, Clone)]
pub struct MigrationLog;

/// The modal's width (logical px).
const WIDTH: f32 = 640.0;
/// The barrier's tint over the rest of the window.
const BACKDROP: Color = Color::srgba(0.0, 0.0, 0.0, 0.45);
/// The body's height limit (% of the window height): with the header, the footer and the barrier's 5% margins the
/// dialog fits an 800 x 500 window; the steps and the log shrink first.
const BODY_MAX_VH: f32 = 70.0;
/// The width of the labels in the summary rows (Project, Version, Git).
const LABEL_WIDTH: f32 = 72.0;

pub struct MigrationPlugin;

impl Plugin for MigrationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, redraw.after(crate::apply_actions));
    }
}

/// One footer button of the modal ([`footer_buttons`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FooterButton {
    pub label: &'static str,
    pub action: MigrationAction,
    /// The primary variant (the phase's main choice).
    pub primary: bool,
    /// `InteractionDisabled`: shown, but not pressable (while the migration runs).
    pub disabled: bool,
    /// Takes the keyboard focus when the modal (re)spawns (`AutoFocus`): the pressable Cancel.
    pub autofocus: bool,
}

impl FooterButton {
    const fn new(label: &'static str, action: MigrationAction, primary: bool, disabled: bool) -> FooterButton {
        let autofocus = matches!(action, MigrationAction::Cancel) && !disabled;
        FooterButton { label, action, primary, disabled, autofocus }
    }
}

/// The footer buttons of `phase`, left to right: Ready → Cancel / Dry run / **Migrate**; Running → the same three,
/// disabled, the last captioned "Migrating..."; Done → **Open project**; Failed → Cancel / Dry run / **Retry**.
/// Cancel has the focus when it is pressable (Ready, Failed); in Done Feathers focuses Open project.
pub fn footer_buttons(phase: &MigrationPhase) -> Vec<FooterButton> {
    use MigrationAction::{Cancel, DryRun, Open, Run};
    match phase {
        MigrationPhase::Ready => vec![
            FooterButton::new("Cancel", Cancel, false, false),
            FooterButton::new("Dry run", DryRun, false, false),
            FooterButton::new("Migrate", Run, true, false),
        ],
        MigrationPhase::Running { .. } => vec![
            FooterButton::new("Cancel", Cancel, false, true),
            FooterButton::new("Dry run", DryRun, false, true),
            FooterButton::new("Migrating...", Run, true, true),
        ],
        MigrationPhase::Done { .. } => vec![FooterButton::new("Open project", Open, true, false)],
        MigrationPhase::Failed { .. } => vec![
            FooterButton::new("Cancel", Cancel, false, false),
            FooterButton::new("Dry run", DryRun, false, false),
            FooterButton::new("Retry", Run, true, false),
        ],
    }
}

/// The git line: its text and colour (a warning when the migration could not be reviewed and reverted with git).
pub fn git_line(git: GitState) -> (String, Color) {
    match git {
        GitState::Clean => ("No uncommitted changes".into(), TEXT_DIM),
        GitState::Dirty { changed } => (
            format!(
                "{} in the project: commit first so the migration can be reviewed and reverted",
                plural(changed, "uncommitted change", "uncommitted changes")
            ),
            WARN,
        ),
        GitState::NotARepo => ("Not a git repository: back up the project first".into(), WARN),
        GitState::Unavailable => ("git is not available: the project's state is unknown".into(), TEXT_DIM),
    }
}

/// The progress of `session`: `(done, total)` file migrations while running and after a successful run.
fn progress(session: &MigrationSession) -> Option<(usize, usize)> {
    match session.phase {
        MigrationPhase::Running { done, total } => Some((done, total)),
        MigrationPhase::Done { written } => Some((written, session.plan.total_stale().max(written))),
        MigrationPhase::Ready | MigrationPhase::Failed { .. } => None,
    }
}

/// Respawns the modal when the session appears, moves or goes.
fn redraw(
    editor: Res<Editor>,
    roots: Query<Entity, With<MigrationRoot>>,
    mut last: Local<Option<(PathBuf, u64, MigrationPhase)>>,
    mut commands: Commands,
) {
    let key = editor.migration.as_ref().map(|s| (s.root.clone(), s.rev, s.phase.clone()));
    if *last == key {
        return;
    }
    *last = key;
    for e in &roots {
        commands.entity(e).try_despawn();
    }
    let Some(session) = &editor.migration else { return };
    let body = body(session);
    let buttons: Vec<Box<dyn Scene>> = footer_buttons(&session.phase).into_iter().map(footer_button).collect();
    commands.spawn_scene(bsn! {
        @FeathersDialog {
            @width: px(WIDTH),
            @contents: bsn_list! {
                @FeathersDialogHeader Children [ @caption("Migrate project") ]
                --
                @FeathersDialogBody
                Node { flex_direction: FlexDirection::Column, row_gap: px(8), max_height: vh(BODY_MAX_VH) }
                Children [ {body} ]
                --
                @FeathersDialogFooter Children [ {buttons} ]
            }
        }
        MigrationRoot
        // The rest of the editor is unavailable (D28): dimmed behind the barrier.
        BackgroundColor(BACKDROP)
    });
}

fn footer_button(b: FooterButton) -> Box<dyn Scene> {
    let variant = if b.primary { ButtonVariant::Primary } else { ButtonVariant::Normal };
    let (label, act) = (b.label, OnActivate(b.action.into()));
    if b.autofocus {
        return Box::new(bsn! {
            @FeathersButton { @variant: variant, @caption: bsn! { @caption(label) } }
            Node { flex_shrink: 0.0 }
            AutoFocus
            act
        });
    }
    if b.disabled {
        Box::new(bsn! {
            @FeathersButton { @variant: variant, @caption: bsn! { @caption(label) } }
            Node { flex_shrink: 0.0 }
            InteractionDisabled
            act
        })
    } else {
        Box::new(bsn! {
            @FeathersButton { @variant: variant, @caption: bsn! { @caption(label) } }
            Node { flex_shrink: 0.0 }
            act
        })
    }
}

/// The modal's body: the summary rows, warnings, an error, the steps, the progress and the log.
fn body(session: &MigrationSession) -> Vec<Box<dyn Scene>> {
    let root = session.root.display().to_string();
    let tip = Tooltip(root.clone());
    let path = bsn! { Node { flex_grow: 1.0, min_width: px(0) } tip Children [ @clipped_text(root, TEXT, 13.0) ] };
    let version = text(format!("{} -> {}", session.from, session.to), TEXT, 13.0);
    let (git, git_color) = git_line(session.git);
    let git = bsn! {
        Node { flex_grow: 1.0, min_width: px(0) }
        Children [ @text(git, git_color, 13.0) ]
    };
    let mut out: Vec<Box<dyn Scene>> = vec![
        Box::new(summary_row("Project", path)),
        Box::new(summary_row("Version", version)),
        Box::new(summary_row("Git", git)),
    ];
    for warning in &session.plan.warnings {
        out.push(Box::new(notice(IconId::Warning, warning.clone(), WARN)));
    }
    if let MigrationPhase::Done { .. } = &session.phase {
        let message =
            format!("The project is at {} and every domain is regenerated. Open project opens it.", session.to);
        out.push(Box::new(notice(IconId::Check, message, TEXT)));
    }
    if let MigrationPhase::Failed { message } = &session.phase {
        let message = format!("Migration failed: {message}. Files already written stay; Retry resumes.");
        out.push(Box::new(notice(IconId::Warning, message, ERR)));
    }
    out.push(Box::new(section("Steps")));
    out.push(Box::new(steps(session)));
    if let Some((done, total)) = progress(session) {
        out.push(Box::new(progress_bar(done, total)));
    }
    out.push(Box::new(section("Log")));
    out.push(Box::new(log(session)));
    out
}

/// A dim label of [`LABEL_WIDTH`] and its value.
fn summary_row(label: &str, value: impl Scene) -> impl Scene {
    let label = label.to_string();
    bsn! {
        Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Start, column_gap: px(8), min_width: px(0) }
        Children [
            @text(label, TEXT_DIM, 13.0) Node { width: px(LABEL_WIDTH), flex_shrink: 0.0 }
            --
            @{value}
        ]
    }
}

/// A warning or error line with its icon.
fn notice(id: IconId, message: String, color: Color) -> impl Scene {
    bsn! {
        Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Start, column_gap: px(6), min_width: px(0) }
        Children [
            @icon(id, 16.0, color) Node { flex_shrink: 0.0 }
            --
            Node { flex_grow: 1.0, min_width: px(0) } Children [ @text(message, color, 13.0) ]
        ]
    }
}

/// A section heading inside the body.
fn section(title: &str) -> impl Scene {
    let title = title.to_string();
    bsn! { @text(title, TEXT, 13.0) Node { margin: UiRect::top(px(4)) } }
}

/// A bordered, scrollable list of `rows` on the view background, `max_height` high at most; in a short window it
/// shrinks down to `min_height` (after the log, which gives way first).
fn list_box(rows: Vec<Box<dyn Scene>>, max_height: f32, min_height: f32) -> impl Scene {
    bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            max_height: px(max_height),
            min_height: px(min_height),
            flex_shrink: 1.0,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(3)),
        }
        BackgroundColor(VIEW_BG)
        BorderColor::all(BORDER)
        Children [ @scroll_column() Children [ {rows} ] ]
    }
}

/// One line of a [`list_box`]: `indent` levels of 16 px.
fn line(s: impl Into<String>, color: Color, indent: u8) -> Box<dyn Scene> {
    let s = s.into();
    let pad = px(4.0 + 16.0 * f32::from(indent));
    Box::new(bsn! { Node { padding: UiRect::left(pad), min_width: px(0) } Children [ @text(s, color, 12.0) ] })
}

/// The steps of the plan: `v2: <title>` (as the CLI's `step v2: <title>`), then per domain `<domain> (N)` and its files.
fn steps(session: &MigrationSession) -> impl Scene {
    let mut rows: Vec<Box<dyn Scene>> = Vec::new();
    if session.plan.is_up_to_date() {
        rows.push(line(format!("The project is up to date ({})", session.plan.to), TEXT_DIM, 0));
    }
    for step in &session.plan.steps {
        rows.push(line(format!("{}: {}", step.to, step.title), TEXT, 0));
        let by_domain = step.by_domain();
        if by_domain.is_empty() {
            rows.push(line("nothing to migrate", TEXT_DIM, 1));
        }
        for (domain, files) in by_domain {
            rows.push(line(format!("{domain} ({})", files.len()), TEXT, 1));
            for file in files {
                rows.push(line(file, TEXT_DIM, 2));
            }
        }
    }
    list_box(rows, 160.0, 44.0)
}

/// The progress of the run: a bar filled `done / total` and the count.
fn progress_bar(done: usize, total: usize) -> impl Scene {
    let fraction = if total == 0 { 1.0 } else { (done as f32 / total as f32).clamp(0.0, 1.0) };
    let label = format!("{done} / {}", plural(total, "file", "files"));
    bsn! {
        Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, column_gap: px(8) }
        Children [
            Node {
                flex_grow: 1.0,
                height: px(8),
                border_radius: BorderRadius::all(px(4)),
                overflow: Overflow::clip(),
            }
            BackgroundColor(VIEW_BG)
            Children [ Node { width: percent(fraction * 100.0), height: percent(100) } BackgroundColor(ACCENT) ]
            --
            @text(label, TEXT_DIM, 12.0) Node { flex_shrink: 0.0 }
        ]
    }
}

/// The session log, oldest first, scrolled to the newest line.
fn log(session: &MigrationSession) -> impl Scene {
    let rows: Vec<Box<dyn Scene>> = if session.log.is_empty() {
        vec![line("Dry run lists the plan here; nothing is written until Migrate.", TEXT_DIM, 0)]
    } else {
        session.log.iter().map(|l| line(l.clone(), log_color(l), 0)).collect()
    };
    bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            height: px(130),
            min_height: px(60),
            // The log gives way before the steps.
            flex_shrink: 4.0,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(3)),
        }
        BackgroundColor(VIEW_BG)
        BorderColor::all(BORDER)
        Children [ @scroll_column() MigrationLog ScrollPosition(Vec2::new(0.0, f32::MAX)) Children [ {rows} ] ]
    }
}

/// A log line's colour: warnings and errors as in the console/logs.
fn log_color(line: &str) -> Color {
    if line.starts_with("error:") {
        ERR
    } else if line.starts_with("warning:") {
        WARN
    } else if line.starts_with("info:") {
        TEXT_DIM
    } else {
        TEXT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(phase: MigrationPhase) -> Vec<(&'static str, bool, bool)> {
        footer_buttons(&phase).into_iter().map(|b| (b.label, b.primary, b.disabled)).collect()
    }

    #[test]
    fn footer_buttons_follow_the_phase() {
        assert_eq!(
            labels(MigrationPhase::Ready),
            [("Cancel", false, false), ("Dry run", false, false), ("Migrate", true, false)]
        );
        assert_eq!(
            labels(MigrationPhase::Running { done: 0, total: 1 }),
            [("Cancel", false, true), ("Dry run", false, true), ("Migrating...", true, true)]
        );
        assert_eq!(labels(MigrationPhase::Done { written: 1 }), [("Open project", true, false)]);
        assert_eq!(
            labels(MigrationPhase::Failed { message: "x".into() }),
            [("Cancel", false, false), ("Dry run", false, false), ("Retry", true, false)]
        );
        let actions: Vec<MigrationAction> =
            footer_buttons(&MigrationPhase::Ready).into_iter().map(|b| b.action).collect();
        assert_eq!(actions, [MigrationAction::Cancel, MigrationAction::DryRun, MigrationAction::Run]);
        assert_eq!(footer_buttons(&MigrationPhase::Done { written: 0 })[0].action, MigrationAction::Open);
        assert_eq!(footer_buttons(&MigrationPhase::Failed { message: String::new() })[2].action, MigrationAction::Run);
    }

    /// Cancel takes the focus whenever it is pressable, never the run button (a stray Enter must not migrate).
    #[test]
    fn cancel_has_the_focus_when_pressable() {
        let focused = |phase: MigrationPhase| -> Vec<&'static str> {
            footer_buttons(&phase).into_iter().filter(|b| b.autofocus).map(|b| b.label).collect()
        };
        assert_eq!(focused(MigrationPhase::Ready), ["Cancel"]);
        assert_eq!(focused(MigrationPhase::Failed { message: "x".into() }), ["Cancel"]);
        assert!(focused(MigrationPhase::Running { done: 0, total: 1 }).is_empty(), "nothing pressable");
        assert!(focused(MigrationPhase::Done { written: 1 }).is_empty(), "Feathers focuses Open project");
    }

    #[test]
    fn git_line_warns_unless_clean() {
        assert_eq!(git_line(GitState::Clean).1, TEXT_DIM);
        let (dirty, color) = git_line(GitState::Dirty { changed: 3 });
        assert!(dirty.starts_with("3 uncommitted changes in the project: commit first"), "{dirty}");
        assert_eq!(color, WARN);
        assert!(git_line(GitState::Dirty { changed: 1 }).0.starts_with("1 uncommitted change in"));
        assert_eq!(git_line(GitState::NotARepo), ("Not a git repository: back up the project first".into(), WARN));
        assert_eq!(git_line(GitState::Unavailable).1, TEXT_DIM);
    }
}
