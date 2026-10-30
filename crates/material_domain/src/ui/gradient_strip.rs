//! Gradient strips (gradient design §8): one slot of a gradient collection rasterized into an `ImageNode` — the bar of
//! the gradients editor content and the rows of the slot lists ([`slot_list_row`]: the gradients editor's, a Sample
//! Gradient's properties). Node previews rasterize with [`strip_pixmap`] too (`ui::gradient_preview`).
//!
//! A strip shows the slot's baked row ([`Slot::bake_row`], what the game samples): its [`SAMPLES`] pixels stretched
//! over the strip's width with linear filtering in linear space (as the GPU filters the sRGB texture), over a
//! checkerboard so alpha shows. An empty slot (or one that does not exist) is drawn hatched.
//!
//! [`GradientStrip`] entities are kept current by [`GradientStripPlugin`]: the collection comes from
//! [`Editor::gradient_collection`] (an open tab's unsaved state first), and a strip is re-rasterized only when its
//! slot's baked row or its size changes.

use bevy::feathers::controls::FeathersListRow;
use bevy::prelude::*;
use resvg::tiny_skia;
use shader_graph_core::gradient::{SAMPLES, Stop, linear_to_srgb, srgb_to_linear};

use super::{TEXT_DIM, text};
use crate::ops::Editor;
use crate::ops::prelude::*;
use crate::util::raster::opaque_image;

/// Checkerboard behind a strip (sRGB bytes), as the image preview's.
const CHECKER: [[u8; 3]; 2] = [[58, 59, 64], [44, 45, 50]];
/// Stripes of an empty slot (sRGB bytes).
const HATCH: [[u8; 3]; 2] = [[44, 44, 48], [38, 38, 41]];

/// An `ImageNode` showing slot `slot` (its [`Slot::id`], never its position) of the gradient collection
/// `collection` (its asset path, e.g. `common/terrain`).
#[derive(Component, Default, Clone, PartialEq, Debug)]
#[require(StripShown)]
pub struct GradientStrip {
    pub collection: String,
    pub slot: u32,
}

/// One row of a slot list (a `FeathersListRow`): `label` (the slot's position) in a narrow column, then a strip with
/// a name over its right end. `row`, `strip` and `name` add components to the row (markers), the strip (a
/// [`GradientStrip`] + `ImageNode`, or a plain background) and the name label (its `Text`; white with a shadow unless
/// it sets its own colours).
pub fn slot_list_row(
    label: String,
    row: Box<dyn Scene>,
    strip: Box<dyn Scene>,
    name: Box<dyn Scene>,
) -> Box<dyn SceneList> {
    bsn! {
        @FeathersListRow
        @{row}
        Node { column_gap: px(6), padding: UiRect::axes(px(4), px(2)) }
        Children [
            Node { width: px(24), flex_shrink: 0.0, justify_content: JustifyContent::End }
            Children [ @text(label, TEXT_DIM, 11.0) ]
            --
            Node { flex_grow: 1.0, height: px(16) }
            Pickable::IGNORE
            Children [
                @{strip}
                Node { position_type: PositionType::Absolute, left: px(0), right: px(0), top: px(0), bottom: px(0) }
                Pickable::IGNORE
                --
                TextFont { font_size: FontSize::Px(12.0) }
                TextColor(Color::WHITE)
                TextShadow { offset: Vec2::splat(1.0), color: Color::BLACK }
                Node { position_type: PositionType::Absolute, right: px(6), top: px(0) }
                Pickable::IGNORE
                @{name}
            ]
        ]
    }
    .into()
}

/// What a strip last drew: the slot's stops (so an unchanged slot is not baked again), its baked row (`None`: empty)
/// and the size, plus the image it owns.
#[derive(Component, Default)]
pub struct StripShown {
    stops: Option<Vec<Stop>>,
    row: Option<Option<Box<[[u8; 4]; SAMPLES]>>>,
    size: UVec2,
    image: Option<Handle<Image>>,
}

pub struct GradientStripPlugin;

impl Plugin for GradientStripPlugin {
    fn build(&self, app: &mut App) {
        // After this frame's edits: a dragged stop shows without a frame of lag.
        app.add_systems(Update, render_strips.after(crate::apply_actions));
    }
}

/// Re-rasterizes every strip whose slot's baked row or size changed.
fn render_strips(
    editor: Res<Editor>,
    mut strips: Query<(Ref<GradientStrip>, &ComputedNode, &mut ImageNode, &mut StripShown)>,
    mut images: ResMut<Assets<Image>>,
) {
    for (strip, node, mut image_node, mut shown) in &mut strips {
        let size = node.size().as_uvec2();
        // Hidden (`Display::None`) or not laid out yet.
        if size.x == 0 || size.y == 0 {
            continue;
        }
        if !editor.is_changed() && !strip.is_changed() && shown.size == size && shown.row.is_some() {
            continue;
        }
        let slot =
            editor.gradient_collection(&strip.collection).and_then(|c| c.slots.iter().find(|s| s.id == strip.slot));
        let stops = slot.map(|s| s.stops.clone());
        if shown.stops == stops && shown.size == size && shown.row.is_some() {
            continue;
        }
        let row = slot.filter(|s| !s.is_empty()).map(|s| Box::new(s.bake_row()));
        shown.stops = stops;
        if shown.row.as_ref() == Some(&row) && shown.size == size {
            continue;
        }
        let pixmap = match &row {
            Some(row) => strip_pixmap(row, size),
            None => empty_pixmap(size),
        };
        shown.row = Some(row);
        shown.size = size;
        let handle = match &shown.image {
            Some(h) => {
                let _ = images.insert(h, opaque_image(pixmap));
                h.clone()
            }
            None => images.add(opaque_image(pixmap)),
        };
        shown.image = Some(handle.clone());
        if image_node.image != handle {
            image_node.image = handle;
        }
    }
}

/// A slot's baked `row` stretched over `size` physical px over a checkerboard: column `x` is `t = x / (width - 1)`,
/// filtered linearly between the row's samples in linear space (as the GPU samples the sRGB texture). Opaque.
pub fn strip_pixmap(row: &[[u8; 4]; SAMPLES], size: UVec2) -> tiny_skia::Pixmap {
    let (w, h) = (size.x.max(1), size.y.max(1));
    let linear = row.map(|p| {
        let c = |i: usize| srgb_to_linear(f32::from(p[i]) / 255.0);
        [c(0), c(1), c(2), f32::from(p[3]) / 255.0]
    });
    let cell = checker_cell(h);
    let mut pixmap = tiny_skia::Pixmap::new(w, h).expect("non-empty strip size");
    let width = w as usize;
    let data = pixmap.data_mut();
    for x in 0..w {
        let t = if w == 1 { 0.0 } else { x as f32 / (w - 1) as f32 };
        let f = t * (SAMPLES - 1) as f32;
        let i = (f.floor() as usize).min(SAMPLES - 2);
        let k = f - i as f32;
        let (a, b) = (linear[i], linear[i + 1]);
        let c: [f32; 4] = std::array::from_fn(|n| a[n] + (b[n] - a[n]) * k);
        let rgb = [0, 1, 2].map(|n| linear_to_srgb(c[n]).clamp(0.0, 1.0) * 255.0);
        for y in 0..h {
            let bg = CHECKER[((x / cell + y / cell) % 2) as usize];
            let px: [u8; 3] = std::array::from_fn(|n| (rgb[n] * c[3] + f32::from(bg[n]) * (1.0 - c[3])).round() as u8);
            let at = (y as usize * width + x as usize) * 4;
            data[at..at + 4].copy_from_slice(&[px[0], px[1], px[2], 255]);
        }
    }
    pixmap
}

/// An empty slot: diagonal stripes over `size` physical px. Opaque.
pub fn empty_pixmap(size: UVec2) -> tiny_skia::Pixmap {
    let (w, h) = (size.x.max(1), size.y.max(1));
    let stripe = (h / 3).clamp(4, 8);
    let mut pixmap = tiny_skia::Pixmap::new(w, h).expect("non-empty strip size");
    let width = w as usize;
    let data = pixmap.data_mut();
    for y in 0..h {
        for x in 0..w {
            let c = HATCH[(((x + y) / stripe) % 2) as usize];
            let at = (y as usize * width + x as usize) * 4;
            data[at..at + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    pixmap
}

/// Checkerboard cell size (physical px) for a strip `h` px tall: a few cells across its height.
fn checker_cell(h: u32) -> u32 {
    (h / 3).clamp(4, 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(p: &tiny_skia::Pixmap, x: u32, y: u32) -> [u8; 4] {
        let c = p.pixel(x, y).unwrap();
        [c.red(), c.green(), c.blue(), c.alpha()]
    }

    #[test]
    fn strip_ends_are_the_row_ends_and_opaque() {
        let mut row = [[0, 0, 0, 255]; SAMPLES];
        row[SAMPLES - 1] = [255, 255, 255, 255];
        let p = strip_pixmap(&row, UVec2::new(300, 12));
        assert_eq!(pixel(&p, 0, 5), [0, 0, 0, 255]);
        assert_eq!(pixel(&p, 299, 5), [255, 255, 255, 255]);
        // Linear filtering between the last two samples happens only in the last 1/127 of the width.
        assert_eq!(pixel(&p, 290, 5), [0, 0, 0, 255]);
    }

    #[test]
    fn transparent_strip_shows_the_checkerboard() {
        let row = [[255, 0, 0, 0]; SAMPLES];
        let p = strip_pixmap(&row, UVec2::new(40, 40));
        let cell = checker_cell(40);
        let [a, b] = CHECKER;
        assert_eq!(pixel(&p, 0, 0), [a[0], a[1], a[2], 255]);
        assert_eq!(pixel(&p, cell, 0), [b[0], b[1], b[2], 255]);
        // Half alpha: half way between the colour and the checker.
        let half = strip_pixmap(&[[255, 0, 0, 128]; SAMPLES], UVec2::new(4, 4));
        let px = pixel(&half, 0, 0);
        assert!(px[0] > 140 && px[0] < 170 && px[1] < 40, "{px:?}");
    }

    #[test]
    fn empty_strip_is_hatched() {
        let p = empty_pixmap(UVec2::new(16, 16));
        let colours: std::collections::BTreeSet<[u8; 4]> =
            (0..16).flat_map(|x| (0..16).map(move |y| (x, y))).map(|(x, y)| pixel(&p, x, y)).collect();
        assert_eq!(colours.len(), 2, "two stripe colours: {colours:?}");
    }
}
