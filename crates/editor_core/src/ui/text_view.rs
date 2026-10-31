//! Read-only text view of a generated file with syntax highlighting (decision 0014, design §6.2).
//!
//! The code is one `Text` whose `TextSpan` children carry the colors, next to a `Text` with the line numbers
//! (same font and size, so the lines match). Spans cannot be selected: Ctrl+C / the Copy button copy the
//! whole file.

use std::collections::HashMap;

use bevy::feathers::constants::fonts;
use bevy::input::mouse::MouseScrollPixelsPerLine;
use bevy::prelude::*;
use bevy::text::FontSource;
use bevy_enhanced_input::prelude::ModKeys;

use super::icon_tool_button;
use super::icons::IconId;
use super::shell::{VIEW_BG, clipped_text};
use crate::action::{Action, TabAction};
use crate::editors::ViewOf;
use crate::editors::text::TEXT as TEXT_VIEW;
use crate::highlight::{Style, highlight, style_color};
use crate::keys::mods;
use crate::state::editor::Editor;
use crate::tab::{TextDoc, format_highlights};
use crate::theme::{ACCENT2, TEXT, TEXT_DIM};

const FONT_SIZE: f32 = 13.0;

/// The editor topbar row of a text tab (path, buttons), rebuilt with the file.
#[derive(Component, Default, Clone)]
pub(crate) struct TextViewBar;

/// Scroll area holding the code.
#[derive(Component, Default, Clone)]
pub struct TextViewCode;

/// A contiguous run of lines `start..=end` (1-based, inclusive) in [`TextViewCode`], tagged with whether its lines
/// are highlighted (`BackgroundColor(ACCENT2)`).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextViewLine {
    pub start: usize,
    pub end: usize,
    pub highlighted: bool,
}

pub struct TextViewPlugin;

impl Plugin for TextViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, redraw.after(crate::apply_actions)).add_observer(scroll_code);
    }
}

pub fn text_view() -> impl Scene {
    bsn! {
        ViewOf(TEXT_VIEW)
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            flex_direction: FlexDirection::Column,
        }
        BackgroundColor(VIEW_BG)
        Visibility::Hidden
        Children [
            TextViewCode
            Node {
                flex_grow: 1.0,
                min_height: px(0),
                flex_direction: FlexDirection::Column,
                overflow: Overflow::scroll(),
                padding: UiRect::new(px(4), px(16), px(8), px(48)),
                align_items: AlignItems::Stretch,
            }
        ]
    }
}

/// Wheel scrolling (Shift+wheel: horizontal). Instead of `ScrollArea`, which scrolls only vertically with a
/// plain wheel.
fn scroll_code(
    mut ev: On<PointerScroll>,
    keys: Res<ButtonInput<KeyCode>>,
    ratio: Res<MouseScrollPixelsPerLine>,
    mut q: Query<(&ComputedNode, &mut ScrollPosition), With<TextViewCode>>,
) {
    let Ok((node, mut pos)) = q.get_mut(ev.entity) else { return };
    ev.propagate(false);
    let px = ev.to_pixels(&ratio);
    let mut delta = Vec2::new(px.x, px.y);
    if mods(&keys).contains(ModKeys::SHIFT) {
        delta = Vec2::new(delta.y, delta.x);
    }
    let max = ((node.content_size() - node.size()) * node.inverse_scale_factor).max(Vec2::ZERO);
    pos.0 = (pos.0 - delta).clamp(Vec2::ZERO, max);
}

fn redraw(
    editor: Res<Editor>,
    assets: Res<AssetServer>,
    bar: Query<Entity, With<TextViewBar>>,
    mut code: Query<(Entity, &mut ScrollPosition), With<TextViewCode>>,
    mut last: Local<Option<(String, u64)>>,
    mut scrolls: Local<HashMap<String, Vec2>>,
    mut commands: Commands,
) {
    let Some(t) = editor.tab().and_then(|t| t.get::<TextDoc>()) else {
        if let (Some((rel, _)), Ok((_, scroll))) = (last.take(), code.single()) {
            scrolls.insert(rel, scroll.0);
        }
        return;
    };
    let key = (t.rel.clone(), t.revision);
    if last.as_ref() == Some(&key) {
        return;
    }
    let (Ok(bar), Ok((code, mut scroll))) = (bar.single(), code.single_mut()) else { return };
    // Each file keeps its scroll position.
    if let Some((rel, _)) = last.as_ref() {
        scrolls.insert(rel.clone(), scroll.0);
    }
    scroll.0 = scrolls.get(&t.rel).copied().unwrap_or_default();
    *last = Some(key);

    commands.entity(bar).despawn_children();
    let (path, open) = (t.rel.clone(), Action::OpenFile(t.path.clone()));
    let source_icon = IconId::File;
    let source = Action::OpenAsset(t.source.clone());
    commands.spawn_scene(bsn! { @clipped_text(path, TEXT, 13.0) ChildOf(bar) });
    // The path keeps its width first: the label yields (clipped) before the path is clipped.
    commands.spawn_scene(
        bsn! { @clipped_text("read-only, generated", TEXT_DIM, 12.0) Node { flex_shrink: 100.0 } ChildOf(bar) },
    );
    if !t.highlights.is_empty() {
        let spec = format_highlights(&t.highlights);
        commands.spawn_scene(bsn! { @clipped_text(spec, TEXT_DIM, 12.0) Node { flex_shrink: 100.0 } ChildOf(bar) });
    }
    commands.spawn_scene(bsn! { Node { flex_grow: 1.0 } ChildOf(bar) });
    let buttons: [Box<dyn Scene>; 3] = [
        Box::new(icon_tool_button(IconId::Copy, "Copy", Action::Tab(TabAction::Copy), "Copy the whole file (Ctrl+C)")),
        Box::new(icon_tool_button(source_icon, "Source", source, "Open the source graph")),
        Box::new(icon_tool_button(IconId::OpenExternal, "Open externally", open, "Open in the default application")),
    ];
    for b in buttons {
        commands.spawn_scene(bsn! { @{b} Node { flex_shrink: 0.0 } ChildOf(bar) });
    }

    commands.entity(code).despawn_children();
    let font = TextFont {
        font: FontSource::Handle(assets.load(fonts::MONO)),
        font_size: FontSize::Px(FONT_SIZE),
        ..default()
    };
    let Some(src) = t.text.as_deref() else {
        commands.spawn((Text::new("The file does not exist (yet)."), font, TextColor(TEXT_DIM), ChildOf(code)));
        return;
    };
    let trimmed = src.trim_end();
    let mut per_line: Vec<Vec<(Style, String)>> = vec![Vec::new()];
    for (style, piece) in highlight(&editor.domain_list, trimmed, t.lang) {
        let mut parts = piece.split('\n');
        if let Some(first) = parts.next()
            && !first.is_empty()
        {
            per_line.last_mut().unwrap().push((style, first.to_string()));
        }
        for next_line in parts {
            per_line.push(Vec::new());
            if !next_line.is_empty() {
                per_line.last_mut().unwrap().push((style, next_line.to_string()));
            }
        }
    }
    let total_lines = per_line.len().max(1);
    let mut runs: Vec<(usize, usize, bool)> = Vec::new();
    for line_no in 1..=total_lines {
        let hl = t.highlights.contains(&line_no);
        if let Some(last) = runs.last_mut()
            && last.2 == hl
        {
            last.1 = line_no;
        } else {
            runs.push((line_no, line_no, hl));
        }
    }
    let num_w = FONT_SIZE * 0.6 * total_lines.to_string().len() as f32 + 12.0;
    for (start_line, end_line, highlighted) in runs {
        let numbers: String = (start_line..=end_line).map(|n| n.to_string()).collect::<Vec<_>>().join("\n");
        let row = commands
            .spawn((
                TextViewLine { start: start_line, end: end_line, highlighted },
                Node {
                    width: percent(100),
                    flex_direction: FlexDirection::Row,
                    column_gap: px(14),
                    padding: UiRect::axes(px(4), px(0)),
                    border_radius: BorderRadius::all(px(2)),
                    ..default()
                },
                BackgroundColor(if highlighted { ACCENT2 } else { Color::NONE }),
                ChildOf(code),
            ))
            .id();
        commands.spawn((
            Text::new(numbers),
            font.clone(),
            TextColor(if highlighted { TEXT } else { TEXT_DIM.with_alpha(0.6) }),
            TextLayout::new(Justify::Right, LineBreak::NoWrap),
            Node { min_width: px(num_w), ..default() },
            ChildOf(row),
        ));
        let text_entity = commands.spawn((Text::default(), font.clone(), TextLayout::no_wrap(), ChildOf(row))).id();
        for (offset, line_spans) in per_line[(start_line - 1)..end_line].iter().enumerate() {
            if offset > 0 {
                commands.spawn((TextSpan::new("\n"), font.clone(), TextColor(TEXT), ChildOf(text_entity)));
            }
            if line_spans.is_empty() {
                commands.spawn((TextSpan::new(" "), font.clone(), TextColor(TEXT), ChildOf(text_entity)));
            } else {
                for (style, piece) in line_spans {
                    commands.spawn((
                        TextSpan::new(piece.clone()),
                        font.clone(),
                        TextColor(style_color(*style)),
                        ChildOf(text_entity),
                    ));
                }
            }
        }
    }
}
