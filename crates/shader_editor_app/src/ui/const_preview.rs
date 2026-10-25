//! Read-only value of the global constant a `Global*` node reads (decision 0017), shown under the node's title:
//! `f32` as a number, `vecN` as a colour square, `bool` / `compileFlag` as a 32×32 square with `0` / `1` and a check
//! mark (green when on, grey when off), "n/a" without a constant of the node's type.
//!
//! Updated from the model whenever it changes (an open table tab's unsaved values first), so it follows a table
//! edited in another tab without canvas rebuilds.

use std::collections::BTreeMap;

use bevy::prelude::*;
use shader_graph_core::consts::{ConstType, GlobalConst};
use shader_graph_core::model::NodeId;
use shader_graph_core::registry::ParamKind;

use super::icons::{IconId, icon};
use super::{TEXT, TEXT_DIM};
use crate::state::Editor;

/// Side of the colour / bool squares (logical px).
const SQUARE: f32 = 32.0;
const ON: Color = Color::srgb_u8(34, 160, 84);
const OFF: Color = Color::srgb_u8(82, 84, 92);

/// What a part of a preview shows; `.0` is the model node.
#[derive(Component, Clone, Default)]
pub enum ConstPart {
    /// The number of an `f32` constant.
    #[default]
    Number,
    /// Colour square of a `vecN`.
    Swatch,
    /// Background square of a flag.
    Flag,
    /// `0` / `1` of a flag.
    Digit,
    /// Check mark of a flag.
    Check,
    /// "n/a" label.
    Na,
}

#[derive(Component, Clone, Default)]
pub struct ConstPreview {
    pub node: NodeId,
    pub part: ConstPart,
}

pub struct ConstPreviewPlugin;

impl Plugin for ConstPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, update_previews.before(bevy::ui::UiSystems::Prepare));
    }
}

/// Type of the constant nodes of this kind read (`None`: not a `Global*` node).
pub fn preview_type(editor: &Editor, kind: &str) -> Option<ConstType> {
    editor.registry.get(kind)?.params().into_iter().find_map(|p| match p.kind {
        ParamKind::GlobalConst(ty) => Some(ty),
        _ => None,
    })
}

fn tag(node: &str, part: ConstPart) -> ConstPreview {
    ConstPreview { node: node.into(), part }
}

/// The preview row of model node `node` reading a constant of type `ty`.
pub fn node_preview(node: &str, ty: ConstType) -> impl Scene {
    let body: Box<dyn Scene> = match ty {
        ConstType::F32 => {
            let t = tag(node, ConstPart::Number);
            Box::new(bsn! { t Text("") TextFont { font_size: FontSize::Px(15.0) } TextColor(TEXT) Pickable::IGNORE })
        }
        ConstType::Vec2 | ConstType::Vec3 | ConstType::Vec4 => {
            let t = tag(node, ConstPart::Swatch);
            Box::new(bsn! {
                t
                Node { width: px(SQUARE), height: px(SQUARE), border: UiRect::all(px(1)), border_radius: BorderRadius::all(px(4)) }
                BackgroundColor(Color::NONE)
                BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.25))
                Pickable::IGNORE
            })
        }
        ConstType::Bool | ConstType::CompileFlag => {
            let (square, digit, check) =
                (tag(node, ConstPart::Flag), tag(node, ConstPart::Digit), tag(node, ConstPart::Check));
            Box::new(bsn! {
                square
                Node {
                    width: px(SQUARE),
                    height: px(SQUARE),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::all(px(4)),
                }
                BackgroundColor(OFF)
                Pickable::IGNORE
                Children [
                    digit
                    Text("0")
                    TextFont { font_size: FontSize::Px(15.0), weight: FontWeight::BOLD }
                    TextColor(Color::WHITE)
                    Pickable::IGNORE
                    --
                    Node { position_type: PositionType::Absolute, right: px(1), top: px(1) }
                    Pickable::IGNORE
                    Children [ @icon(IconId::Check, 11.0, Color::WHITE) check ]
                ]
            })
        }
    };
    let na = tag(node, ConstPart::Na);
    bsn! {
        Node { min_height: px(SQUARE + 4.0), align_items: AlignItems::Center, column_gap: px(8) }
        Pickable::IGNORE
        Children [
            @{body}
            --
            na
            Text("n/a")
            TextFont { font_size: FontSize::Px(12.0) }
            TextColor(TEXT_DIM)
            Visibility::Hidden
            Pickable::IGNORE
        ]
    }
}

/// The constant node `node` reads, if it resolves to one of the node's type.
fn resolve(editor: &Editor, consts: &BTreeMap<String, GlobalConst>, node: &str) -> Option<GlobalConst> {
    let n = editor.doc()?.asset.nodes.get(node)?;
    let def = editor.registry.get(&n.kind)?;
    let params = editor.registry.resolved_params(&n.kind, &n.params);
    let (id, ty) = def.params().into_iter().find_map(|p| match p.kind {
        ParamKind::GlobalConst(ty) => Some((p.id, ty)),
        _ => None,
    })?;
    let c = consts.get(params.get(&id)?.as_str())?;
    (c.ty == ty).then(|| c.clone())
}

/// `x` with at most 3 decimals and no trailing zeros.
fn short(x: f32) -> String {
    let s = format!("{x:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" { "0".into() } else { s.into() }
}

fn update_previews(
    editor: Res<Editor>,
    added: Query<(), Added<ConstPreview>>,
    mut parts: Query<(
        &ConstPreview,
        Option<&mut Text>,
        Option<&mut BackgroundColor>,
        Option<&mut ImageNode>,
        Option<&mut Visibility>,
    )>,
) {
    if !editor.is_changed() && added.is_empty() {
        return;
    }
    let consts: BTreeMap<String, GlobalConst> =
        editor.global_consts().into_iter().map(|(name, (_, c))| (name, c)).collect();
    let mut cache: BTreeMap<NodeId, Option<GlobalConst>> = BTreeMap::new();
    for (p, text, bg, image, vis) in &mut parts {
        let c = cache.entry(p.node.clone()).or_insert_with(|| resolve(&editor, &consts, &p.node));
        let on = c.as_ref().is_some_and(|c| c.value.as_bool());
        // The squares go away with the constant ("n/a" takes their place).
        let shown = if c.is_some() { Visibility::Inherited } else { Visibility::Hidden };
        match p.part {
            ConstPart::Number => {
                if let Some(mut t) = text {
                    let want = c.as_ref().map_or(String::new(), |c| short(c.value.as_f32()));
                    if t.0 != want {
                        t.0 = want;
                    }
                }
            }
            ConstPart::Swatch => {
                if let Some(mut v) = vis {
                    v.set_if_neq(shown);
                }
                if let Some(mut bg) = bg {
                    let want = c.as_ref().map_or(Color::NONE, |c| {
                        let f = c.value.floats();
                        let at = |i: usize| f.get(i).copied().unwrap_or(0.0);
                        Color::linear_rgb(at(0), at(1), at(2))
                    });
                    bg.set_if_neq(BackgroundColor(want));
                }
            }
            ConstPart::Flag => {
                if let Some(mut v) = vis {
                    v.set_if_neq(shown);
                }
                if let Some(mut bg) = bg {
                    let want = match c {
                        Some(_) if on => ON,
                        Some(_) => OFF,
                        None => Color::NONE,
                    };
                    bg.set_if_neq(BackgroundColor(want));
                }
            }
            ConstPart::Digit => {
                if let Some(mut t) = text {
                    let want = match c {
                        Some(_) if on => "1",
                        Some(_) => "0",
                        None => "",
                    };
                    if t.0 != want {
                        t.0 = want.into();
                    }
                }
            }
            ConstPart::Check => {
                if let Some(mut image) = image {
                    let alpha = match c {
                        Some(_) if on => 1.0,
                        Some(_) => 0.3,
                        None => 0.0,
                    };
                    if image.color.alpha() != alpha {
                        image.color = Color::WHITE.with_alpha(alpha);
                    }
                }
            }
            ConstPart::Na => {
                if let Some(mut v) = vis {
                    v.set_if_neq(if c.is_some() { Visibility::Hidden } else { Visibility::Inherited });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_short() {
        assert_eq!(short(0.35), "0.35");
        assert_eq!(short(1.0), "1");
        assert_eq!(short(-0.0001), "0");
        assert_eq!(short(2.5), "2.5");
    }
}
