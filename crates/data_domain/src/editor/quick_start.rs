//! Data's step of Help → Quick Start (design §8.2, ADR 0049): querying `Data<'w>` in-game and async `.qsd` loading.

use bevy::prelude::*;
use editor_core::domain::QuickStartDef;
use editor_core::theme::{TEXT, TEXT_DIM, text};
use editor_core::ui::welcome::code;

/// Data's Quick Start ([`super::DataDomain`] registers it).
pub const QUICK_START: QuickStartDef = QuickStartDef { steps: &QUICK_START_STEPS, body: step_body };

/// Data's sub-sections in the Quick Start dialog: `(title, optional)`.
pub const QUICK_START_STEPS: [(&str, bool); 1] = [("Querying", false)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Block {
    Para(&'static str),
    Note(&'static str),
    Rust(&'static str),
}

fn step_body(step: usize) -> Vec<Box<dyn Scene>> {
    step_blocks(step)
        .into_iter()
        .map(|b| -> Box<dyn Scene> {
            match b {
                Block::Para(s) => Box::new(text(s, TEXT, 13.0)),
                Block::Note(s) => Box::new(text(s, TEXT_DIM, 12.0)),
                Block::Rust(s) => Box::new(code(s, Some(editor_core::highlight::highlight_rust))),
            }
        })
        .collect()
}

#[cfg(test)]
fn step_texts(step: usize) -> Vec<&'static str> {
    let text = |b| match b {
        Block::Para(s) | Block::Note(s) | Block::Rust(s) => s,
    };
    step_blocks(step).into_iter().map(text).collect()
}

fn step_blocks(_step: usize) -> Vec<Block> {
    let (p, note, rs) = (Block::Para, Block::Note, Block::Rust);
    vec![
        p("Every .collection.ron table generates a statically typed CollectionHandle field on the Data SystemParam. \
           Iterate rows directly in a for-loop or read 1-row config tables with .first():"),
        rs("use bevy::prelude::*;\n\
            use quillshade::prelude::*;\n\n\
            fn inspect_data(data: Data) {\n\
            \x20   if !data.is_ready() {\n\
            \x20       return;\n\
            \x20   }\n\n\
            \x20   for (id, book) in data.collection_library_books_core {\n\
            \x20       info!(\"{}: {} ({} pages)\", id, book.title, book.pages);\n\
            \x20   }\n\n\
            \x20   if let Some(cfg) = data.collection_config.first() {\n\
            \x20       info!(\"title={} max_players={}\", cfg.game_title, cfg.max_players);\n\
            \x20   }\n\
            }"),
        p("Each collection handle also exposes its compile-time row count .len, .iter(), and O(1) lookup by row UUID \
           with .get(uuid):"),
        rs("let total = data.collection_library_books_core.len; // compile-time row count\n\
            for (id, row) in data.collection_library_books_core.iter() {\n\
            \x20   let same = data.collection_library_books_core.get(id); // O(1) lookup by UUID\n\
            }"),
        note(
            "Important (async loading): Collection handles always exist at compile time (no Option<Collection>), \
             and .len is known immediately, but baked .collection.qsd row data loads asynchronously via Bevy's \
             AssetServer (iterating before load finishes yields 0 rows). Either check data.is_ready() before \
             querying, or gate gameplay behind a loading screen using QuillshadeLoader and QuillshadeReady.",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::ui::welcome::code_spans;

    #[test]
    fn quick_start_has_one_querying_step() {
        assert_eq!(QUICK_START_STEPS, [("Querying", false)]);
        for step in 0..QUICK_START_STEPS.len() {
            assert!(!step_texts(step).is_empty(), "step {step} has a body");
        }
    }

    #[test]
    fn quick_start_rust_snippets_are_highlighted() {
        let mut rust = 0;
        for step in 0..QUICK_START_STEPS.len() {
            for b in step_blocks(step) {
                let Block::Rust(s) = b else { continue };
                let colours: Vec<Color> =
                    code_spans(s, Some(editor_core::highlight::highlight_rust)).into_iter().map(|(_, c)| c).collect();
                rust += 1;
                assert!(colours.iter().any(|c| *c != TEXT), "step {step}: not highlighted: {s}");
            }
        }
        assert!(rust > 0, "the Quick Start shows Rust code");
    }
}
