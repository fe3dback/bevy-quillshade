//! Material's steps of Help → Quick Start (design §8.2): from the generated crate hooked into the game (core's steps
//! before these) to a material on screen (README "Hooking it up to a game"); core draws the dialog, the step tabs and
//! headings ([`QuickStartDef`]).

use bevy::prelude::*;

use crate::ui::{TEXT, TEXT_DIM, text};
use editor_core::domain::{Highlighter, QuickStartDef};
use editor_core::ui::welcome::code;

/// Material's Quick Start ([`crate::domain::MaterialDomain`] registers it).
pub const QUICK_START: QuickStartDef = QuickStartDef { steps: &QUICK_START_STEPS, body: step_body };

/// Material's step after core's (the generated crate in Cargo.toml, `QuillshadePlugin`, `QuillshadeLoader`):
/// `(title, optional)`; the dialog titles it `"Material: Spawn"`.
pub const QUICK_START_STEPS: [(&str, bool); 1] = [("Spawn", false)];

/// One block of a Quick Start step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Block {
    /// A paragraph.
    Para(&'static str),
    /// A dimmed remark.
    Note(&'static str),
    /// A code snippet.
    Code(&'static str, CodeLang),
}

/// The lines of step `step` below its heading.
fn step_body(step: usize) -> Vec<Box<dyn Scene>> {
    step_blocks(step)
        .into_iter()
        .map(|b| -> Box<dyn Scene> {
            match b {
                Block::Para(s) => Box::new(text(s, TEXT, 13.0)),
                Block::Note(s) => Box::new(text(s, TEXT_DIM, 12.0)),
                Block::Code(s, lang) => Box::new(code(s, lang.highlighter())),
            }
        })
        .collect()
}

/// Every text of step `step` (paragraphs, notes and code), in order.
#[cfg(test)]
fn step_texts(step: usize) -> Vec<&'static str> {
    let text = |b| match b {
        Block::Para(s) | Block::Note(s) | Block::Code(s, _) => s,
    };
    step_blocks(step).into_iter().map(text).collect()
}

fn step_blocks(_step: usize) -> Vec<Block> {
    let (p, note) = (Block::Para, Block::Note);
    let rs = |s: &'static str| Block::Code(s, CodeLang::Rust);
    vec![
        p("QuillshadePlugin registers every generated material type, reserves a handle per material \
           (BakedMaterials) and loads the baked values and textures (BakedTextures). Every material is a field of \
           the BakedMaterials resource, by its path under quillshade/material/ \
           (quillshade/material/env/rock_wet.material.ron is baked.env.rock_wet). The handles exist from the \
           start; the materials appear behind them on their own, with the values set in the graph."),
        rs("use quillshade::material::{BakedMaterials, BakedTextures};\n\n\
           // .add_systems(Startup, spawn) next to QuillshadePlugin\n\
           fn spawn(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, baked: Res<BakedMaterials>) {\n\
           \x20   // Pbr3d needs tangents for a Normal input.\n\
           \x20   let sphere = Sphere::new(1.0).mesh().uv(48, 24).with_generated_tangents().unwrap();\n\
           \x20   commands.spawn((Mesh3d(meshes.add(sphere)), MeshMaterial3d(baked.env.rock_wet.clone())));\n\
           \x20   commands.spawn((Camera3d::default(), Transform::from_xyz(0.0, 1.0, 4.0)));\n\
           }\n\n\
           // Unlit3d: MeshMaterial3d(baked.fx.hologram.clone()) as well.\n\
           // Sprite2d: (Sprite::from_image(image), SpriteMaterial(baked.ui.dissolve.clone())).\n\
           // By id, e.g. from your own asset data (None for an unknown id or another type):\n\
           // baked.get::<RockWetMaterial>(\"env/rock_wet\"), tex.get(\"textures/rock_albedo\")."),
        note(
            "To change a value, edit the graph: the game picks it up (Hot reload). The material constructors \
              are internal to the generated crate.",
        ),
    ]
}

/// How a [`code`] block is coloured.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodeLang {
    /// One colour (TOML, shell, directory trees).
    Plain,
    /// The generated-Rust tab's lexer and colours.
    Rust,
}

impl CodeLang {
    /// Rust goes through the generated-Rust tab's lexer and colours.
    fn highlighter(self) -> Option<Highlighter> {
        match self {
            CodeLang::Plain => None,
            CodeLang::Rust => Some(editor_core::highlight::highlight_rust),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::ui::welcome::code_spans;

    /// The crate, plugin, and loader steps are core's (design §8.2): Material has a single `"Spawn"` step.
    #[test]
    fn quick_start_has_one_step() {
        assert_eq!(QUICK_START_STEPS, [("Spawn", false)]);
        for step in 0..QUICK_START_STEPS.len() {
            assert!(!step_texts(step).is_empty(), "step {step} has a body");
        }
    }

    #[test]
    fn quick_start_has_no_cli_commands() {
        for step in 0..QUICK_START_STEPS.len() {
            for t in step_texts(step) {
                for banned in ["regen", "check --compile", "Game Assets"] {
                    assert!(!t.contains(banned), "step {step} mentions {banned:?}: {t}");
                }
            }
        }
    }

    /// Every code block is coloured as its step data tags it: a Rust block through the lexer (several colours), a
    /// plain one (TOML, shell, directory trees) as one span.
    #[test]
    fn quick_start_rust_snippets_are_highlighted() {
        let mut rust = 0;
        for step in 0..QUICK_START_STEPS.len() {
            for b in step_blocks(step) {
                let Block::Code(s, lang) = b else { continue };
                let colours: Vec<Color> = code_spans(s, lang.highlighter()).into_iter().map(|(_, c)| c).collect();
                match lang {
                    CodeLang::Rust => {
                        rust += 1;
                        assert!(colours.iter().any(|c| *c != TEXT), "step {step}: not highlighted: {s}");
                    }
                    CodeLang::Plain => assert_eq!(colours, vec![TEXT], "step {step}: one plain span: {s}"),
                }
            }
        }
        assert!(rust > 0, "the Quick Start shows Rust code");
    }
}
