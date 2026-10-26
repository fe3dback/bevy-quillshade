//! Read-only value of the global constant a `Global*` node reads (decision 0017), shown under the node's title:
//! `f32` as a number, `vecN` as a colour square, `bool` / `compileFlag` as a 32×32 square with `0` / `1` and a check
//! mark (green when on, grey when off), "n/a" without a constant of the node's type.
//!
//! Updated from the model whenever it changes (an open table tab's unsaved values first), so it follows a table
//! edited in another tab without canvas rebuilds.

use bevy::ecs::system::SystemParamItem;
use bevy::prelude::*;
use shader_graph_core::consts::{ConstType, GlobalConst};
use shader_graph_core::model::NodeId;
use shader_graph_core::registry::{NodeDef, ParamKind};

use super::TEXT;
use super::icons::{IconId, icon};
use super::node_preview::{NodePreview, PreviewTarget};
use crate::doc::Doc;
use crate::state::Editor;

/// Side of the colour / bool squares (logical px).
const SQUARE: f32 = 32.0;
const ON: Color = Color::srgb_u8(34, 160, 84);
const OFF: Color = Color::srgb_u8(82, 84, 92);

/// What a part of a preview row shows.
#[derive(Component, Clone, Copy, Default)]
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
}

/// Preview of the constant a `Global*` node reads.
pub struct ConstPreview;

impl NodePreview for ConstPreview {
    /// The constant, when it resolves to one of the node's type.
    type Key = GlobalConst;
    type Draw = (
        Query<'static, 'static, &'static Children>,
        Query<
            'static,
            'static,
            (
                &'static ConstPart,
                Option<&'static mut Text>,
                Option<&'static mut BackgroundColor>,
                Option<&'static mut ImageNode>,
                Option<&'static mut Visibility>,
            ),
        >,
    );

    fn applies(def: &dyn NodeDef) -> bool {
        const_type(def).is_some()
    }

    fn layout(_: &dyn NodeDef) -> Node {
        Node { min_height: px(SQUARE + 4.0), align_items: AlignItems::Center, column_gap: px(8), ..default() }
    }

    fn content(def: &dyn NodeDef) -> Box<dyn Scene> {
        match const_type(def).unwrap_or(ConstType::F32) {
            ConstType::F32 => Box::new(
                bsn! { ConstPart::Number Text("") TextFont { font_size: FontSize::Px(15.0) } TextColor(TEXT) Pickable::IGNORE },
            ),
            ConstType::Vec2 | ConstType::Vec3 | ConstType::Vec4 => Box::new(bsn! {
                ConstPart::Swatch
                Node { width: px(SQUARE), height: px(SQUARE), border: UiRect::all(px(1)), border_radius: BorderRadius::all(px(4)) }
                BackgroundColor(Color::NONE)
                BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.25))
                Pickable::IGNORE
            }),
            ConstType::Bool | ConstType::CompileFlag => Box::new(bsn! {
                ConstPart::Flag
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
                    ConstPart::Digit
                    Text("0")
                    TextFont { font_size: FontSize::Px(15.0), weight: FontWeight::BOLD }
                    TextColor(Color::WHITE)
                    Pickable::IGNORE
                    --
                    Node { position_type: PositionType::Absolute, right: px(1), top: px(1) }
                    Pickable::IGNORE
                    Children [ @icon(IconId::Check, 11.0, Color::WHITE) ConstPart::Check ]
                ]
            }),
        }
    }

    fn key(editor: &Editor, doc: &Doc, node: &NodeId) -> Option<GlobalConst> {
        let n = doc.asset.nodes.get(node)?;
        let def = editor.registry.get(&n.kind)?;
        let params = editor.registry.resolved_params(&n.kind, &n.params);
        let (id, ty) = def.params().into_iter().find_map(|p| match p.kind {
            ParamKind::GlobalConst(ty) => Some((p.id, ty)),
            _ => None,
        })?;
        let name = params.get(&id)?.as_str().to_string();
        let (_, c) = editor.global_consts().remove(&name)?;
        (c.ty == ty).then_some(c)
    }

    fn draw(c: Option<&GlobalConst>, at: PreviewTarget, (children, parts): &mut SystemParamItem<Self::Draw>) -> bool {
        let on = c.is_some_and(|c| c.value.as_bool());
        // The squares go away with the constant ("n/a" takes their place).
        let shown = if c.is_some() { Visibility::Inherited } else { Visibility::Hidden };
        for e in children.iter_descendants(at.root) {
            let Ok((part, text, bg, image, vis)) = parts.get_mut(e) else { continue };
            match part {
                ConstPart::Number => {
                    if let Some(mut t) = text {
                        let want = c.map_or(String::new(), |c| short(c.value.as_f32()));
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
                        let want = c.map_or(Color::NONE, |c| {
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
            }
        }
        true
    }
}

/// Type of the constant nodes of this definition read (`None`: not a `Global*` node).
fn const_type(def: &dyn NodeDef) -> Option<ConstType> {
    def.params().into_iter().find_map(|p| match p.kind {
        ParamKind::GlobalConst(ty) => Some(ty),
        _ => None,
    })
}

/// `x` with at most 3 decimals and no trailing zeros.
fn short(x: f32) -> String {
    let s = format!("{x:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" { "0".into() } else { s.into() }
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
