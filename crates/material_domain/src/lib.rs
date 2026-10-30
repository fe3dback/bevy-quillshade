//! The Material domain of the editor (design §9.1): material graphs, functions, curves, constant tables, Texture
//! assets and gradient collections, with their editor kinds, views and the bevy_noodle canvas; registered by
//! [`MaterialDomain`]. The editor itself (state, dispatch, window shell) is `editor_core`.
//!
//! The graph canvas, its gestures and palette are `node_graph_base`'s, behind [`graph_domain::MaterialGraph`].
//!
//! Data flow: widgets push [`ops::Action`]s → `editor_core::apply_actions` mutates the `Editor` (documents are the
//! source of truth) → the canvas and panels redraw from it. Material's `Editor` methods are the extension traits in
//! [`ops`] (`use crate::ops::prelude::*`).

#![allow(clippy::type_complexity)] // Bevy system signatures

use bevy::prelude::*;

pub mod autolayout;
pub mod canvas;
pub use editor_core::config;
pub mod consts_doc;
pub mod curve;
mod demo;
pub mod doc;
pub mod doc_kind;
pub mod domain;
pub mod editors;
pub mod gradients_doc;
pub mod graph_domain;
pub mod highlight_wesl;
pub mod icons;
pub use editor_core::history;
pub mod inline;
pub mod material_state;
pub use editor_core::keys;
pub mod ops;
pub mod quick_start;
pub mod search;
pub mod tab;
#[cfg(test)]
mod tests;
pub mod texture;
pub mod ui;
pub use editor_core::util;

use canvas::{CanvasSystems, EditorCanvas};
pub use domain::{MaterialDomain, material_only, test_editor};
use editor_core::apply_actions;
use material_state::MaterialStates;
use ops::prelude::*;
use ops::{Action, Actions, Editor};

/// Material's editor logic (`DomainRegistrar::logic`, also headless; the canvas sync is `node_graph_base`'s): the
/// canvas following finished drags, live validation.
pub(crate) fn material_logic(app: &mut App) {
    app.add_systems(
        Update,
        (follow_moves.after(apply_actions).before(CanvasSystems), validate_when_idle.after(CanvasSystems)),
    );
}

/// Material's window parts (`DomainRegistrar::bevy`): the editor content's views, previews and the debug hooks (the
/// palette and the canvas gestures are `node_graph_base`'s, the curve editor `curve_kit`'s, the Texture and image
/// tabs `texture_kit`'s).
pub(crate) fn material_window(app: &mut App) {
    app.add_plugins((
        ui::consts_view::ConstsViewPlugin,
        ui::node_preview::NodePreviewsPlugin,
        ui::props::PropsPlugin,
        ui::gradient_strip::GradientStripPlugin,
        ui::gradient_view::GradientViewPlugin,
    ))
    .add_systems(Update, (debug_screenshot, demo::scripted_input));
}

/// Seconds of quiet before live validation runs.
const VALIDATE_DELAY: f32 = 0.25;

/// Finished drags update the model without rebuilding the canvas: a canvas showing the graph as it was before a
/// [`ops::GraphEdit::Moved`] shows it as it is after.
fn follow_moves(mut editor: ResMut<Editor>, ec: Option<ResMut<EditorCanvas>>) {
    if editor.domains.material().moves.is_empty() {
        return;
    }
    let moves = std::mem::take(&mut editor.bypass_change_detection().domains.material_mut().moves);
    let Some(mut ec) = ec else { return };
    for (before, after) in moves {
        if ec.shown == before {
            ec.shown = after;
        }
    }
}

fn validate_when_idle(mut editor: ResMut<Editor>, time: Res<Time>, mut quiet: Local<f32>) {
    if !editor.bypass_change_detection().needs_validation() {
        *quiet = 0.0;
        return;
    }
    *quiet += time.delta_secs();
    if *quiet >= VALIDATE_DELAY {
        editor.validate();
        *quiet = 0.0;
    }
}

/// Debug aid, Material's half (core takes the shot, `editor_core::app`): with `SHADER_EDITOR_SCREENSHOT` set,
/// `SHADER_EDITOR_OPEN=env/rock_wet.material.ron` opens an asset at startup (`…ron#wesl` / `#rust`: its generated
/// file in a text tab, after the asset's graph; `@game/textures/rock_albedo.ktx2`: a game image's tab;
/// `@internal/default`: a built-in texture's read-only Texture tab); `SHADER_EDITOR_SELECT=stdlib::Lerp` selects the
/// first node of that kind, or the comment frame of that id (`c01`); on a gradient collection the slot at a position,
/// `0:s1` also its stop 1, `0:m1` its midpoint 1.
/// `SHADER_EDITOR_DEMO=palette|wire|dialog|texture_channel:N|game_file_menu[:2]|gradient_color` additionally opens
/// that popup before the shot (`game_file_menu`: the RMB menu of the first game image with a Texture asset — its item
/// disabled —, `:2` of the first without one; `gradient_color`: selects the first stop of a gradients tab's selected
/// slot and opens its color picker).
fn debug_screenshot(
    mut frames: Local<u32>,
    mut editor: ResMut<Editor>,
    mut palette: ResMut<ui::palette::Palette>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    if std::env::var_os("SHADER_EDITOR_SCREENSHOT").is_none() {
        return;
    }
    *frames += 1;
    if *frames == 2
        && let Ok(open) = std::env::var("SHADER_EDITOR_OPEN")
        && let Some(path) = open.strip_prefix(texture::GAME_KEY_PREFIX)
    {
        editor.open_game_image(path);
    } else if *frames == 2
        && let Ok(open) = std::env::var("SHADER_EDITOR_OPEN")
        && open.starts_with(shader_graph_core::texture::INTERNAL_PREFIX)
    {
        editor.open_asset(&shader_project::AssetId::new(shader_project::AssetType::Texture, open));
    } else if *frames == 2
        && let Ok(open) = std::env::var("SHADER_EDITOR_OPEN")
        && let (rel, lang) = open.split_once('#').map_or((open.as_str(), None), |(r, l)| (r, Some(l)))
        && let Some(id) = shader_project::assets::id_from_rel(rel)
    {
        editor.open_asset(&id);
        if let Some(lang) = lang.and_then(|l| [tab::WESL, tab::RUST].into_iter().find(|k| *k == l)) {
            editor.show_generated(&id, lang);
        }
        if let Ok(sel) = std::env::var("SHADER_EDITOR_SELECT")
            && let Some(d) = editor.doc_mut()
        {
            // A node kind, or the id of a comment frame.
            if let Some(n) = d.asset.nodes.iter().find(|(_, n)| n.kind == sel).map(|(k, _)| k.clone()) {
                d.set_selection([n]);
            } else if d.asset.comments.iter().any(|f| f.id == sel) {
                d.selected_frame = Some(sel);
            }
            d.revision += 1;
            editor.ui.props += 1;
        } else if let Ok(sel) = std::env::var("SHADER_EDITOR_SELECT")
            && editor.gradients_doc().is_some()
        {
            // `<slot>`, `<slot>:s<stop>` or `<slot>:m<segment>`.
            let (slot, item) = sel.split_once(':').map_or((sel.as_str(), None), |(s, i)| (s, Some(i)));
            if let Ok(slot) = slot.parse() {
                editor.apply(Action::from(ops::GradientsAction::SelectSlot(slot)));
            }
            let pick = |prefix| item.and_then(|i| i.strip_prefix(prefix)).and_then(|i| i.parse().ok());
            let sel = match (pick('s'), pick('m')) {
                (Some(i), _) => Some(gradients_doc::GradientSel::Stop(i)),
                (_, Some(i)) => Some(gradients_doc::GradientSel::Mid(i)),
                _ => None,
            };
            editor.apply(Action::from(ops::GradientsAction::Select(sel)));
        }
    }
    if *frames == 150 {
        let output = editor
            .doc()
            .and_then(|d| d.asset.nodes.iter().find(|(k, _)| editor.is_output_node(k)).map(|(k, _)| k.clone()));
        match std::env::var("SHADER_EDITOR_DEMO").as_deref() {
            Ok("palette") => palette.show(Vec2::new(520.0, 300.0), Vec2::ZERO, None),
            Ok("wire") => {
                let link = output.map(|node| canvas::PinKey { node, pin: "roughness".into(), output: false });
                palette.show(Vec2::new(520.0, 300.0), Vec2::ZERO, link);
            }
            Ok("dialog") => actions
                .push(Action::NewAssetDialog { kind: (shader_project::layout::MATERIAL, "material"), template: 0 }),
            Ok(d) if d.starts_with("texture_channel:") => {
                actions.push(texture::PreviewChannel::new(d["texture_channel:".len()..].parse::<usize>().ok()))
            }
            Ok("gradient_color") => {
                // The first stop of the selected slot (`SHADER_EDITOR_SELECT=<slot>` picks one with stops).
                let first = Some(gradients_doc::GradientSel::Stop(0));
                editor.apply(Action::from(ops::GradientsAction::Select(first)));
                ui::gradient_view::open_stop_color(&mut commands, 0);
            }
            Ok(d) if d == "game_file_menu" || d == "game_file_menu:2" => {
                // The RMB menu of the first game image with a Texture asset (disabled item) or, `:2`, without one.
                let has = |f: &&String| {
                    shader_project::game_assets::texture_for_file(&editor.domains.material().disk, f).is_some()
                };
                let file = if d.ends_with(":2") {
                    editor.game_files.iter().find(|f| !has(f))
                } else {
                    editor.game_files.iter().find(has)
                };
                if let Some(f) = file {
                    let row = ui::tree::TreeRow {
                        tree: ui::tree::TreeId::GameAssets,
                        item: ui::tree::TreeItem::GameFile(f.clone()),
                    };
                    ui::dialog::open_menu_entries(
                        &mut commands,
                        Vec2::new(240.0, 300.0),
                        ui::tree::row_menu(&editor, &row),
                    );
                }
            }
            _ => {}
        }
    }
}
