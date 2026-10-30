//! Help → Quick Start (design §8.2): core's steps first — the generated crate `crates/quillshade` as a workspace
//! member and dependency of the game, then `QuillshadePlugin` —, then each domain's steps ([`QuickStartDef`],
//! registration order), titled with the domain's name. The dialog ([`crate::ui::welcome::quick_start`]) shows one step
//! at a time.

use bevy::prelude::*;
use project_core::DomainId;

use crate::domain::{DomainList, Highlighter};
use crate::theme::{TEXT, TEXT_DIM, text};
use crate::ui::welcome::code;

/// Core's steps as `(title, optional)`; the first one is the crate step.
pub const CORE_STEPS: [(&str, bool); 2] = [("Crate", false), ("Plugin", false)];

/// One step of the Quick Start dialog.
#[derive(Clone)]
pub struct Step {
    /// `None`: core's.
    pub domain: Option<DomainId>,
    /// A domain's step reads `"{domain title}: {title}"`.
    pub title: String,
    pub optional: bool,
    body: fn(usize) -> Vec<Box<dyn Scene>>,
    /// The step's index among its owner's steps (the argument of `body`).
    index: usize,
}

impl Step {
    /// The lines below the step's heading.
    pub fn body(&self) -> Vec<Box<dyn Scene>> {
        (self.body)(self.index)
    }
}

/// Every step of the dialog: core's, then each domain's.
pub fn steps(list: &DomainList) -> Vec<Step> {
    let core = CORE_STEPS.iter().enumerate().map(|(index, (title, optional))| Step {
        domain: None,
        title: title.to_string(),
        optional: *optional,
        body: core_body,
        index,
    });
    let domains = list.quick_starts().flat_map(|(id, def)| {
        let name = list.get(id).map_or(id.as_str(), |d| d.info.title);
        def.steps.iter().enumerate().map(move |(index, (title, optional))| Step {
            domain: Some(id),
            title: format!("{name}: {title}"),
            optional: *optional,
            body: def.body,
            index,
        })
    });
    core.chain(domains).collect()
}

/// One block of a core step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Block {
    /// A paragraph.
    Para(&'static str),
    /// A dimmed remark.
    Note(&'static str),
    /// A code snippet: `true` Rust (highlighted), `false` plain (TOML).
    Code(&'static str, bool),
}

fn core_body(step: usize) -> Vec<Box<dyn Scene>> {
    let rust: Highlighter = crate::highlight::highlight_rust;
    core_blocks(step)
        .into_iter()
        .map(|b| -> Box<dyn Scene> {
            match b {
                Block::Para(s) => Box::new(text(s, TEXT, 13.0)),
                Block::Note(s) => Box::new(text(s, TEXT_DIM, 12.0)),
                Block::Code(s, is_rust) => Box::new(code(s, is_rust.then_some(rust))),
            }
        })
        .collect()
}

fn core_blocks(step: usize) -> Vec<Block> {
    let (p, note) = (Block::Para, Block::Note);
    match step {
        0 => vec![
            p("File > New Project... on the game crate (the directory with its Cargo.toml) creates quillshade/ and \
               assets/ there: quillshade/project.ron holds the project settings, quillshade/<domain>/ each domain's \
               sources. Every save rewrites the generated Rust crate crates/quillshade/ (do not edit it)."),
            p("Add the generated crate to the game's Cargo.toml (the editor never edits it):"),
            Block::Code(
                "[workspace]\n\
                 members = [\"crates/quillshade\"]\n\n\
                 [dependencies]\n\
                 bevy = { version = \"0.20\", features = [\"file_watcher\"] }   # file_watcher: hot reload\n\
                 quillshade = { path = \"crates/quillshade\" }",
                false,
            ),
            note(
                "If Bevy is declared in [workspace.dependencies], crates/quillshade takes it with \
                  bevy = { workspace = true }, so the game's Bevy version and features apply.",
            ),
        ],
        _ => vec![
            p("Add QuillshadePlugin after DefaultPlugins: it adds the plugin of every domain the project uses. The \
               next steps show what each domain gives the game."),
            Block::Code(
                "use bevy::prelude::*;\n\n\
                 fn main() {\n\
                 \x20   App::new()\n\
                 \x20       .add_plugins((DefaultPlugins, quillshade::QuillshadePlugin))\n\
                 \x20       .run();\n\
                 }",
                true,
            ),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::welcome::code_spans;

    #[test]
    fn core_steps_have_bodies_without_cli_commands_or_domain_paths() {
        for step in 0..CORE_STEPS.len() {
            let blocks = core_blocks(step);
            assert!(!blocks.is_empty(), "step {step} has a body");
            for b in blocks {
                let (Block::Para(s) | Block::Note(s) | Block::Code(s, _)) = b;
                for banned in ["regen", "check --compile", "material", "shaders"] {
                    assert!(!s.contains(banned), "step {step} mentions {banned:?}: {s}");
                }
            }
        }
    }

    #[test]
    fn core_rust_snippet_is_highlighted() {
        let rust = core_blocks(1).into_iter().find_map(|b| match b {
            Block::Code(s, true) => Some(s),
            _ => None,
        });
        let spans = code_spans(rust.expect("a Rust snippet"), Some(crate::highlight::highlight_rust));
        assert!(spans.iter().any(|(_, c)| *c != TEXT), "highlighted");
    }

    #[test]
    fn no_domain_steps_without_registrations() {
        let list = DomainList::new(Vec::new());
        let titles: Vec<String> = steps(&list).into_iter().map(|s| s.title).collect();
        assert_eq!(titles, ["Crate", "Plugin"]);
    }
}
