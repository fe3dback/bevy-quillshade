//! Light's steps of Help → Quick Start (design §8.2): environments, transition curves, cube-map textures, the
//! `Light` `SystemParam`, and live hot-reload.

use bevy::prelude::*;
use editor_core::domain::{Highlighter, QuickStartDef};
use editor_core::theme::{TEXT, TEXT_DIM, text};
use editor_core::ui::welcome::code;

/// Light's Quick Start ([`super::LightDomain`] registers it).
pub const QUICK_START: QuickStartDef = QuickStartDef { steps: &QUICK_START_STEPS, body: step_body };

/// Light's sub-sections in the Quick Start dialog: `(title, optional)`.
pub const QUICK_START_STEPS: [(&str, bool); 1] = [("Transitions", false)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Block {
    Para(&'static str),
    Note(&'static str),
    Code(&'static str, CodeLang),
}

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
        p("QuillshadePlugin registers the Light runtime, spawns managed directional lights, and drives your \
           Camera3d's skybox, image-based lighting (IBL), distance fog, exposure, tonemapping, and color grading \
           through the Light SystemParam:"),
        rs("use std::time::Duration;\n\
           use bevy::prelude::*;\n\
           use quillshade::light::{Curve, Light, LightTransitionFinished, Preset, State};\n\n\
           // .add_systems(Startup, setup).add_systems(Update, cycle_weather)\n\
           fn setup(mut commands: Commands, mut light: Light) {\n\
           \x20   commands.spawn((Camera3d::default(), Transform::from_xyz(0.0, 2.0, 6.0)));\n\
           \x20   light.set_immediate(Preset::Day);\n\
           }\n\n\
           fn cycle_weather(\n\
           \x20   keys: Res<ButtonInput<KeyCode>>,\n\
           \x20   mut light: Light,\n\
           \x20   mut finished: MessageReader<LightTransitionFinished>,\n\
           ) {\n\
           \x20   if keys.just_pressed(KeyCode::Space) {\n\
           \x20       light.transition_to(Preset::Sunset, Curve::SmoothStep, Duration::from_secs(2));\n\
           \x20   }\n\
           \x20   for ev in finished.read() {\n\
           \x20       info!(\"settled on preset {:?}\", ev.preset);\n\
           \x20   }\n\
           }"),
        note(
            "State is a Bevy States enum (State::None, State::Transitioning, and one variant per environment \
              preset) so you can schedule systems with in_state(State::Day) or OnEnter(State::Night).",
        ),
    ]
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodeLang {
    Plain,
    Rust,
}

impl CodeLang {
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

    #[test]
    fn quick_start_has_one_step() {
        assert_eq!(QUICK_START_STEPS, [("Transitions", false)]);
        for step in 0..QUICK_START_STEPS.len() {
            assert!(!step_texts(step).is_empty(), "step {step} has a body");
        }
    }

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
