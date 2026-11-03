//! Rich help text (node descriptions): a small line-based markup rendered as `Text`s with `TextSpan` children.
//!
//! One block per line:
//! - `# Heading` — a bold heading (`# Output: **Float** \`0..1\``),
//! - `- item` — a bullet item (inputs, outputs, modes),
//! - `! note` — an important note, bright with an accent bar,
//! - `> example` — a small dim example,
//! - an empty line — a gap,
//! - anything else — a dim paragraph.
//!
//! Inline, in every block: `**bold**` and `` `code` `` (types, ranges, expressions; in the code view's type color). An
//! unclosed marker is kept as text. The UI font is an ASCII subset: write `*`, `->`, `-` instead of `×`, `→`, `−`.

use crate::highlight::{Style, style_color};
use crate::theme::{ACCENT, TEXT, TEXT_DIM};
use bevy::feathers::constants::fonts;
use bevy::prelude::*;
use bevy::text::FontSourceTemplate;

/// Kind of one line of the markup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Block {
    Heading,
    Bullet,
    Note,
    Example,
    Paragraph,
    Gap,
}

/// A run of inline text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub code: bool,
}

/// The blocks of `src` (lines trimmed); consecutive empty lines collapse into one [`Block::Gap`], leading and trailing ones are
/// dropped.
pub fn parse(src: &str) -> Vec<(Block, Vec<Span>)> {
    let mut out: Vec<(Block, Vec<Span>)> = Vec::new();
    for line in src.lines() {
        // Indentation is ignored, so the markup can live in indented raw strings.
        let line = line.trim();
        let (block, rest) = if line.trim().is_empty() {
            (Block::Gap, "")
        } else if let Some(r) = line.strip_prefix("# ") {
            (Block::Heading, r)
        } else if let Some(r) = line.strip_prefix("- ") {
            (Block::Bullet, r)
        } else if let Some(r) = line.strip_prefix("! ") {
            (Block::Note, r)
        } else if let Some(r) = line.strip_prefix("> ") {
            (Block::Example, r)
        } else {
            (Block::Paragraph, line)
        };
        if block == Block::Gap && out.last().is_none_or(|b| b.0 == Block::Gap) {
            continue;
        }
        out.push((block, inline(rest)));
    }
    if out.last().is_some_and(|b| b.0 == Block::Gap) {
        out.pop();
    }
    out
}

/// Inline runs of one line: `**bold**` and `` `code` `` (code wins inside bold: `` **`x`** `` is bold code).
fn inline(s: &str) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let (mut bold, mut cur) = (false, String::new());
    let mut rest = s;
    let flush = |out: &mut Vec<Span>, cur: &mut String, bold: bool, code: bool| {
        if !cur.is_empty() {
            out.push(Span { text: std::mem::take(cur), bold, code });
        }
    };
    while let Some(c) = rest.chars().next() {
        if let Some(r) = rest.strip_prefix("**")
            && (bold || r.contains("**"))
        {
            flush(&mut out, &mut cur, bold, false);
            bold = !bold;
            rest = r;
        } else if c == '`'
            && let Some((code, r)) = rest[1..].split_once('`')
        {
            flush(&mut out, &mut cur, bold, false);
            out.push(Span { text: code.to_string(), bold, code: true });
            rest = r;
        } else {
            cur.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    flush(&mut out, &mut cur, bold, false);
    out
}

/// `src` without markup: the text of every block, one per line (search, tooltips).
pub fn plain(src: &str) -> String {
    let lines: Vec<String> = parse(src)
        .into_iter()
        .map(|(block, spans)| {
            let text: String = spans.into_iter().map(|s| s.text).collect();
            if block == Block::Bullet { format!("• {text}") } else { text }
        })
        .collect();
    lines.join("\n")
}

/// The rendered markup: a column of blocks, as wide as its parent (the text wraps).
pub fn rich_text(src: &str) -> impl Scene {
    let blocks: Vec<Box<dyn Scene>> = parse(src).into_iter().map(|(b, spans)| block(b, spans)).collect();
    bsn! {
        Node { flex_direction: FlexDirection::Column, row_gap: px(3) }
        Children [ {blocks} ]
    }
}

fn block(kind: Block, spans: Vec<Span>) -> Box<dyn Scene> {
    let (size, color, bold) = match kind {
        Block::Heading => (12.0, TEXT, true),
        Block::Note => (12.0, TEXT, false),
        Block::Example => (11.0, TEXT_DIM, false),
        Block::Bullet | Block::Paragraph => (12.0, TEXT_DIM, false),
        Block::Gap => return Box::new(bsn! { Node { height: px(4) } }),
    };
    let line = line(spans, size, color, bold);
    match kind {
        // The bullet hangs: wrapped lines start under the text, not under the dot.
        Block::Bullet => Box::new(bsn! {
            Node { flex_direction: FlexDirection::Row, column_gap: px(5), padding: UiRect::left(px(4)) }
            Children [
                // Not in the default font's subset.
                Text("•")
                TextFont { font: FontSourceTemplate::Handle(fonts::MONO), font_size: FontSize::Px(size) }
                TextColor(TEXT_DIM)
                --
                Node { flex_grow: 1.0, flex_shrink: 1.0, min_width: px(0) }
                Children [ {vec![line]} ]
            ]
        }),
        Block::Note => Box::new(bsn! {
            Node { padding: UiRect::left(px(6)), border: UiRect::left(px(2)) }
            BorderColor::all(ACCENT)
            Children [ {vec![line]} ]
        }),
        _ => line,
    }
}

/// One `Text` of `spans`: bold runs in [`TEXT`], code runs in the code view's type color, the rest in `color`.
fn line(spans: Vec<Span>, size: f32, color: Color, bold: bool) -> Box<dyn Scene> {
    let spans: Vec<Box<dyn Scene>> = spans
        .into_iter()
        .map(|s| -> Box<dyn Scene> {
            let weight = if bold || s.bold { FontWeight::BOLD } else { FontWeight::NORMAL };
            let c = match (s.code, s.bold) {
                (true, _) => style_color(Style::Type),
                (false, true) => TEXT,
                (false, false) => color,
            };
            let text = s.text;
            Box::new(bsn! {
                TextSpan(text)
                TextFont { font_size: FontSize::Px(size), weight: weight }
                TextColor(c)
            })
        })
        .collect();
    Box::new(bsn! {
        Text("")
        TextFont { font_size: FontSize::Px(size) }
        TextColor(color)
        Pickable { should_block_lower: false, is_hoverable: false }
        Children [ {spans} ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(text: &str, bold: bool, code: bool) -> Span {
        Span { text: text.into(), bold, code }
    }

    #[test]
    fn blocks_by_line_prefix() {
        let b: Vec<Block> = parse("\n# H\n  - a\n\n  \n! n\n    > e\np\n\n").into_iter().map(|b| b.0).collect();
        use Block::*;
        assert_eq!(b, vec![Heading, Bullet, Gap, Note, Example, Paragraph]);
    }

    #[test]
    fn inline_bold_and_code() {
        assert_eq!(
            inline("a **b `c`** `d` e"),
            vec![
                span("a ", false, false),
                span("b ", true, false),
                span("c", true, true),
                span(" ", false, false),
                span("d", false, true),
                span(" e", false, false)
            ]
        );
        // Unclosed markers stay text.
        assert_eq!(inline("x ** y ` z"), vec![span("x ** y ` z", false, false)]);
    }

    #[test]
    fn plain_strips_markup() {
        assert_eq!(plain("# Out: **Float** `0..1`\n- `UV`: uv0"), "Out: Float 0..1\n• UV: uv0");
    }
}
