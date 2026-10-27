//! Read-only view of a game image (design §4.4): the image in the preview shared with the Texture tab
//! ([`super::image_preview`], design §6.2: zoom, pan, fit, channels, Tile 3x3) and an editor topbar with the path,
//! `WxH | format | mips` ([`crate::game_image::describe`]) and the preview's controls; a load / decode error is shown
//! as text instead of the image. No sampler settings: those belong to a Texture asset.

use std::path::PathBuf;

use bevy::prelude::*;

use super::image_preview::{PreviewInfoText, preview_area, preview_controls};
use super::{TEXT, TEXT_DIM, VIEW_BG, ViewOf, text};
use crate::doc_kind::TabDoc;
use crate::state::Editor;
use crate::tab::{Tab, ViewKind};

/// The editor topbar row of an image tab (path, info, the preview's controls), rebuilt with the image's tab.
#[derive(Component, Default, Clone)]
pub(crate) struct ImageViewBar;

pub struct ImageViewPlugin;

impl Plugin for ImageViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, redraw_bar.after(crate::apply_actions));
    }
}

pub fn image_view() -> impl Scene {
    bsn! {
        ViewOf(ViewKind::Image)
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
        Children [ @preview_area(ViewKind::Image) ]
    }
}

/// Rebuilds the editor topbar when another image tab is active, its file changed or its channel changed (the
/// buttons show the choice; Tile 3x3 shows its own state).
fn redraw_bar(
    editor: Res<Editor>,
    bar: Query<Entity, With<ImageViewBar>>,
    mut last: Local<Option<(PathBuf, String, u64, Option<usize>)>>,
    mut commands: Commands,
) {
    let Ok(bar) = bar.single() else { return };
    let Some(Tab::Image(doc)) = editor.tab() else {
        *last = None;
        return;
    };
    let Some(project) = &editor.project else { return };
    // The root too: another project's tab of the same path may be restored without a frame in between.
    let key = (project.root.clone(), doc.path.clone(), doc.revision, doc.channel);
    if last.as_ref() == Some(&key) {
        return;
    }
    *last = Some(key);
    commands.entity(bar).despawn_children();
    let mut controls: Vec<Box<dyn Scene>> = vec![
        Box::new(text(doc.base_title(), TEXT, 13.0)),
        Box::new(bsn! { @text("", TEXT_DIM, 12.0) PreviewInfoText }),
        Box::new(bsn! { Node { flex_grow: 1.0 } }),
    ];
    controls.extend(preview_controls(doc));
    for c in controls {
        commands.spawn_scene(bsn! { @{c} ChildOf(bar) });
    }
}
