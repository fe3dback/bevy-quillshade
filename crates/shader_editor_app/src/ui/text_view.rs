//! Read-only text view of generated WESL/Rust with syntax highlighting (decision 0014).
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

use super::icons::IconId;
use super::{TEXT, TEXT_DIM, VIEW_BG, ViewOf, clipped_text, icon_tool_button, text};
use crate::editors::main_input::mods;
use crate::highlight::{Style, highlight};
use crate::state::{Action, Editor, GenFile};
use crate::tab::{Lang, Tab, ViewKind};

const FONT_SIZE: f32 = 13.0;

/// The editor topbar row of a text tab (path, buttons), rebuilt with the file.
#[derive(Component, Default, Clone)]
pub struct TextViewBar;

/// Scroll area holding the code.
#[derive(Component, Default, Clone)]
pub struct TextViewCode;

pub struct TextViewPlugin;

impl Plugin for TextViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, redraw).add_observer(scroll_code);
    }
}

pub fn text_view() -> impl Scene {
    bsn! {
        ViewOf(ViewKind::Text)
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
                overflow: Overflow::scroll(),
                padding: UiRect::new(px(4), px(16), px(8), px(48)),
                align_items: AlignItems::Start,
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

pub fn style_color(style: Style) -> Color {
    match style {
        Style::Plain => TEXT,
        Style::Comment => Color::srgb_u8(0x7f, 0x84, 0x8e),
        Style::String => Color::srgb_u8(0x98, 0xc3, 0x79),
        Style::Number => Color::srgb_u8(0xd1, 0x9a, 0x66),
        Style::Keyword => Color::srgb_u8(0xc6, 0x78, 0xdd),
        Style::Type => Color::srgb_u8(0xe5, 0xc0, 0x7b),
        Style::Function => Color::srgb_u8(0x61, 0xaf, 0xef),
        Style::Attribute | Style::Macro => Color::srgb_u8(0x56, 0xb6, 0xc2),
        Style::Punct => Color::srgb_u8(0xab, 0xb2, 0xbf),
    }
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
    let Some(Tab::Text(t)) = editor.tab() else { return };
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
    let which = match t.lang {
        Lang::Wesl => GenFile::Wesl,
        Lang::Rust => GenFile::Rust,
    };
    let (path, open) = (t.rel.clone(), Action::ShowFile(t.source.clone(), which));
    let (source_icon, source) = (IconId::for_id(&t.source, None), Action::OpenAsset(t.source.clone()));
    commands.spawn_scene(bsn! { @clipped_text(path, TEXT, 13.0) ChildOf(bar) });
    let layout = TextLayout::no_wrap();
    commands.spawn_scene(
        bsn! { @text("read-only, generated", TEXT_DIM, 12.0) layout Node { flex_shrink: 0.0 } ChildOf(bar) },
    );
    commands.spawn_scene(bsn! { Node { flex_grow: 1.0 } ChildOf(bar) });
    let buttons: [Box<dyn Scene>; 3] = [
        Box::new(icon_tool_button(IconId::Copy, "Copy", Action::Copy, "Copy the whole file (Ctrl+C)")),
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
    let lines = src.lines().count().max(1);
    let numbers: String = (1..=lines).map(|n| format!("{n}\n")).collect();
    let row = commands
        .spawn((Node { flex_direction: FlexDirection::Row, column_gap: px(14), ..default() }, ChildOf(code)))
        .id();
    commands.spawn((
        Text::new(numbers.trim_end()),
        font.clone(),
        TextColor(TEXT_DIM.with_alpha(0.6)),
        TextLayout::new(Justify::Right, LineBreak::NoWrap),
        Node { min_width: px(FONT_SIZE * 0.6 * lines.to_string().len() as f32 + 12.0), ..default() },
        ChildOf(row),
    ));
    let text = commands.spawn((Text::default(), font.clone(), TextLayout::no_wrap(), ChildOf(row))).id();
    for (style, piece) in highlight(src.trim_end(), t.lang) {
        commands.spawn((TextSpan::new(piece), font.clone(), TextColor(style_color(style)), ChildOf(text)));
    }
}
