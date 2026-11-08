//! Welcome screen, shown over the panels while no project is open (decision 0018): the logo, recent projects
//! (a click opens one), Open / New project, and how to hook the generated crate into the game. Also the Quick Start
//! dialog's body ([`quick_start`], design §8.2).

use std::path::PathBuf;

use bevy::feathers::constants::fonts;
use bevy::feathers::controls::{ButtonVariant, FeathersButton};
use bevy::prelude::*;
use bevy::text::FontSourceTemplate;
use bevy::ui_widgets::ScrollArea;

use crate::action::{Action, Dialog, Submit};
use crate::highlight::style_color;
use crate::logo::logo;
use crate::theme::{TEXT, TEXT_DIM, text};
use crate::tooltip::Tooltip;
use crate::ui::icons::{IconId, icon};
use crate::ui::widgets::{Caption, GroupButton, button_group};

use super::shell::VIEW_BG;
use super::{OnActivate, button, icon_button, icon_tool_button};
use crate::domain::Highlighter;
use crate::quick_start::Step;
use crate::state::editor::Editor;

#[derive(Component, Default, Clone)]
pub struct Welcome;

/// A list of recent projects that [`redraw_recent`] fills: on the welcome screen and in the Open project dialog.
#[derive(Component, Default, Clone)]
pub struct RecentList {
    /// How many of the most recent projects it shows.
    pub limit: usize,
    /// Rows show only the project name (the full path is the tooltip), not a second line with the path.
    pub compact: bool,
}

pub struct WelcomePlugin;

impl Plugin for WelcomePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (show_welcome, redraw_recent.after(super::dialog::redraw)).after(crate::apply_actions));
    }
}

/// Fills the area below the main menu; hidden while a project is open. `rust` colours its Rust snippet.
pub fn welcome(rust: Option<Highlighter>) -> impl Scene {
    let version = format!("Version {}", crate::EDITOR_VERSION);
    bsn! {
        Welcome
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            overflow: Overflow::scroll_y(),
            padding: UiRect::axes(px(16), px(32)),
        }
        ScrollArea
        BackgroundColor(VIEW_BG)
        ZIndex(10)
        Visibility::Hidden
        Children [
            @logo(256.0)
            --
            @text(crate::EDITOR_TITLE, TEXT, 30.0) Node { margin: UiRect::top(px(8)) }
            --
            @text(version, TEXT_DIM, 13.0)
            --
            @text(crate::DISCLAIMER, TEXT_DIM, 11.0)
            Node { max_width: px(560), margin: UiRect::new(px(0), px(0), px(10), px(28)) }
            TextLayout { justify: Justify::Center }
            --
            Node { flex_direction: FlexDirection::Row, column_gap: px(40), align_items: AlignItems::Start }
            Children [
                Node { width: px(380), flex_direction: FlexDirection::Column, row_gap: px(6) }
                Children [
                    @text("Recent projects", TEXT, 16.0)
                    --
                    RecentList { limit: {crate::config::RECENT_LIMIT}, compact: false }
                    Node { flex_direction: FlexDirection::Column, row_gap: px(2) }
                ]
                --
                Node { width: px(460), flex_direction: FlexDirection::Column, row_gap: px(8) }
                Children [
                    @text("Start", TEXT, 16.0)
                    --
                    Node { flex_direction: FlexDirection::Row, column_gap: px(8) }
                    Children [
                        @button("Open Project...", Action::OpenProjectDialog, false)
                        --
                        @button("New Project...", Action::NewProjectDialog, true)
                    ]
                    --
                    {setup_help(rust)}
                ]
            ]
        ]
    }
}

/// How to hook the generated crate into the game (`Cargo.toml` dependency and `QuillshadePlugin`).
fn setup_help(rust: Option<Highlighter>) -> Vec<Box<dyn Scene>> {
    vec![
        Box::new(bsn! {
            Text("After creating a project, add the generated crate to Cargo.toml:")
            TextFont { font_size: FontSize::Px(12.0) }
            TextColor(TEXT_DIM)
        }),
        Box::new(code("[dependencies]\nquillshade = { path = \"crates/quillshade\" }", None)),
        Box::new(bsn! {
            Text("And enable the plugin in the app:")
            TextFont { font_size: FontSize::Px(12.0) }
            TextColor(TEXT_DIM)
        }),
        Box::new(code("app.add_plugins(quillshade::QuillshadePlugin);", rust)),
    ]
}

/// Body of the Quick Start dialog showing step `step` of `steps` (clamped):
/// - Row 1: common setup button group (`Crate`, `Plugin`, `Loader`) + domain switcher button group (`Material`, `Light`, `Data`).
/// - Row 2 (when a domain step is active): that domain's sub-sections button group (`Files`, `Spawn`, ...).
/// - Step heading and body lines ([`crate::quick_start::Step::body`]).
pub fn quick_start(steps: Vec<Step>, step: usize) -> impl Scene {
    let step = step.min(steps.len().saturating_sub(1));
    let active_domain = steps.get(step).and_then(|s| s.domain);

    let core_tabs: Vec<GroupButton> = steps
        .iter()
        .enumerate()
        .filter(|(_, s)| s.domain.is_none())
        .map(|(i, s)| step_tab(i, &s.section_title, s.optional, i == step))
        .collect();

    let mut domain_tabs: Vec<GroupButton> = Vec::new();
    let mut seen_domains = Vec::new();
    for (i, s) in steps.iter().enumerate() {
        let Some(dom) = s.domain else { continue };
        if seen_domains.contains(&dom) {
            continue;
        }
        seen_domains.push(dom);
        let label = s.domain_title.clone().unwrap_or_else(|| dom.as_str().to_string());
        let tip = format!("{label} domain Quick Start");
        domain_tabs.push(GroupButton {
            caption: Caption::Text(label),
            action: Action::QuickStart(i),
            selected: active_domain == Some(dom),
            disabled: false,
            tip,
        });
    }

    let mut nav_rows: Vec<Box<dyn Scene>> = Vec::new();
    let mut top_groups: Vec<Box<dyn Scene>> = Vec::new();
    if !core_tabs.is_empty() {
        top_groups.push(Box::new(button_group(core_tabs)));
    }
    if !domain_tabs.is_empty() {
        top_groups.push(Box::new(button_group(domain_tabs)));
    }
    nav_rows.push(Box::new(bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            column_gap: px(12),
            row_gap: px(6),
            align_items: AlignItems::Center,
        }
        Children [ {top_groups} ]
    }));

    if let Some(dom) = active_domain {
        let sub_tabs: Vec<GroupButton> = steps
            .iter()
            .enumerate()
            .filter(|(_, s)| s.domain == Some(dom))
            .map(|(i, s)| {
                let label =
                    if s.optional { format!("{} (optional)", s.section_title) } else { s.section_title.clone() };
                let tip = format!("Step {}: {}", i + 1, s.title);
                GroupButton {
                    caption: Caption::Text(label),
                    action: Action::QuickStart(i),
                    selected: i == step,
                    disabled: false,
                    tip,
                }
            })
            .collect();
        if sub_tabs.len() > 1 {
            nav_rows.push(Box::new(button_group(sub_tabs)));
        }
    }

    let mut lines: Vec<Box<dyn Scene>> = Vec::new();
    if let Some(s) = steps.get(step) {
        let mut heading = format!("Step {} of {}: {}", step + 1, steps.len(), s.title);
        if s.optional {
            heading.push_str(" (optional)");
        }
        lines.push(Box::new(text(heading, TEXT, 15.0)));
        lines.extend(s.body());
    }
    bsn! {
        Node { flex_direction: FlexDirection::Column, row_gap: px(12) }
        Children [
            Node { flex_direction: FlexDirection::Column, row_gap: px(6) }
            Children [ {nav_rows} ]
            --
            // A fixed minimum height: the dialog does not jump between short and long steps.
            Node { flex_direction: FlexDirection::Column, row_gap: px(8), min_height: px(440) }
            Children [ {lines} ]
        ]
    }
}

/// One button of the step tabs (a joined button group); an optional step says so.
fn step_tab(i: usize, title: &str, optional: bool, selected: bool) -> GroupButton {
    let label = if optional { format!("{}. {title} (optional)", i + 1) } else { format!("{}. {title}", i + 1) };
    let tip = format!("Step {}: {title}", i + 1);
    GroupButton { caption: Caption::Text(label), action: Action::QuickStart(i), selected, disabled: false, tip }
}

/// The coloured pieces of a [`code`] block: through `highlight` (a domain's lexer, decision 0014, in the text tabs'
/// colours), else one plain span (TOML, shell, directory trees).
pub fn code_spans(s: &'static str, highlight: Option<Highlighter>) -> Vec<(&'static str, Color)> {
    match highlight {
        None => vec![(s, TEXT)],
        Some(h) => h(s).into_iter().map(|(style, piece)| (piece, style_color(style))).collect(),
    }
}

/// A monospace code block; the pieces of [`code_spans`] are `TextSpan`s of one `Text`, with a Copy button on the
/// top-right.
pub fn code(s: &'static str, highlight: Option<Highlighter>) -> impl Scene {
    let spans: Vec<Box<dyn Scene>> = code_spans(s, highlight)
        .into_iter()
        .map(|(piece, color)| -> Box<dyn Scene> {
            Box::new(bsn! {
                TextSpan(piece)
                TextFont { font: FontSourceTemplate::Handle(fonts::MONO), font_size: FontSize::Px(12.0) }
                TextColor(color)
            })
        })
        .collect();
    let copy = icon_tool_button(IconId::Copy, "Copy", Action::CopyText(s.to_string()), "Copy code to clipboard");
    bsn! {
        Node {
            width: percent(100),
            padding: UiRect::axes(px(10), px(6)),
            border_radius: BorderRadius::all(px(4)),
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Start,
            column_gap: px(8),
        }
        BackgroundColor(Color::srgb_u8(36, 38, 44))
        Children [
            Node { flex_grow: 1.0, min_width: px(0) }
            Text("")
            TextFont { font: FontSourceTemplate::Handle(fonts::MONO), font_size: FontSize::Px(12.0) }
            TextColor(TEXT)
            Pickable::IGNORE
            Children [ {spans} ]
            --
            @{copy}
        ]
    }
}

/// One recent project: a click on the name opens it, the "x" on the right forgets it (the files stay).
fn recent_row(path: PathBuf, compact: bool) -> impl Scene {
    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    let (full, tip) = (path.display().to_string(), Tooltip(format!("Open {}", path.display())));
    let lines: Vec<Box<dyn Scene>> = if compact {
        vec![Box::new(text(name, TEXT, 13.0))]
    } else {
        vec![Box::new(text(name, TEXT, 14.0)), Box::new(text(full, TEXT_DIM, 12.0))]
    };
    let forget = icon_button(IconId::Close, Action::ForgetRecent(path.clone()), "Remove from recent projects");
    let act = OnActivate(Action::OpenProject(path));
    let pad = if compact { px(4) } else { px(5) };
    // A plain Feathers button of one or two lines: as tall as its text, left-aligned.
    bsn! {
        Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, column_gap: px(2) }
        Children [
            @FeathersButton {
                @variant: ButtonVariant::Plain,
                @caption: bsn_list! {
                    @icon(IconId::Folder, 16.0, TEXT)
                    --
                    Node { flex_direction: FlexDirection::Column, min_width: px(0) }
                    Children [ {lines} ]
                },
            }
            Node {
                height: Val::Auto,
                flex_grow: 1.0,
                min_width: px(0),
                justify_content: JustifyContent::Start,
                column_gap: px(10),
                padding: UiRect::axes(px(8), pad),
            }
            act
            tip
            --
            @{forget}
        ]
    }
}

fn show_welcome(editor: Res<Editor>, mut q: Query<&mut Visibility, With<Welcome>>) {
    let want = if editor.project.is_none() { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut q {
        v.set_if_neq(want);
    }
}

/// Refills every [`RecentList`] when the recent projects change, and a new one (the dialog's) at once.
fn redraw_recent(
    editor: Res<Editor>,
    lists: Query<(Entity, &RecentList)>,
    added: Query<(), Added<RecentList>>,
    mut last: Local<Option<Vec<PathBuf>>>,
    mut commands: Commands,
) {
    let recent = &editor.config.recent_projects;
    if last.as_ref() == Some(recent) && added.is_empty() {
        return;
    }
    *last = Some(recent.clone());
    for (list, RecentList { limit, compact }) in &lists {
        // The compact list belongs to the Open project dialog (`DialogRoot`), which `dialog::redraw` despawns when
        // the dialog closes (e.g. when a recent project row in it is clicked).
        if *compact && !matches!(editor.dialog, Some(Dialog::Input { submit: Submit::OpenProject, .. })) {
            continue;
        }
        commands.entity(list).despawn_children();
        if recent.is_empty() {
            commands.spawn_scene(bsn! { @text("No recent projects yet.", TEXT_DIM, 13.0) ChildOf(list) });
        }
        for p in recent.iter().take(*limit).cloned() {
            let compact = *compact;
            commands.spawn_scene(bsn! { @recent_row(p, compact) ChildOf(list) });
        }
    }
}
