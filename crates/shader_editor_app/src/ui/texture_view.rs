//! Texture asset tab (design §4.4): the settings form on the left (path, loader settings, sampler), the preview on
//! the right, decoded through [`GameImageCache`] with the document's current (unsaved) settings. The mouse wheel
//! zooms around the pointer, a drag pans, a double click fits the image again; "Tile 3x3" repeats the image so the
//! address modes show, R | G | B | All shows one channel in grey or the image. Easy mode (decision 0028) is
//! only the use-case presets in the header, without the form; Advanced is the form with every field. Built-in
//! textures (`@internal/…`) show their values as text.
//!
//! Form widgets are the Properties ones ([`PropField`] + `PropTarget::Texture`), so edits take the usual `SetValue` /
//! `SetComponent` path; the path field applies on Enter (`TextureAction::SetPath`, refused when another Texture
//! asset points at the file). The form is rebuilt only when [`TextureDoc::layout`] changes.

use bevy::feathers::controls::{FeathersTextInput, FeathersTextInputContainer};
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::MouseScrollPixelsPerLine;
use bevy::input_focus::FocusedInput;
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::ui::ui_transform::UiGlobalTransform;
use shader_graph_core::Value;

use super::dialog::InitText;
use super::image_view::Checker;
use super::props::{checkbox_w, integer, labeled, number, section, select_w};
use super::{HEADER_BG, TEXT, TEXT_DIM, VIEW_BG, ViewOf, chip, scroll_column, text};
use crate::doc_kind::TabDoc;
use crate::game_image::{GameImageCache, describe, previewable};
use crate::state::{Action, Actions, Editor, PropTarget, TextureAction};
use crate::tab::ViewKind;
use crate::texture_doc::FilterPreset;
use crate::texture_doc::{TextureDoc, TextureField};

/// Space kept around the fitted image (logical px).
const PAD: f32 = 16.0;
/// Width of the form column.
const FORM_W: f32 = 390.0;
const ERROR: Color = Color::srgb(0.95, 0.42, 0.42);
const WARN: Color = Color::srgb(0.95, 0.78, 0.35);
/// Zoom limits (image px per logical px).
const ZOOM_MIN: f32 = 1.0 / 16.0;
const ZOOM_MAX: f32 = 64.0;

/// Header row (Easy / Advanced, presets, channel, tiling, zoom), rebuilt with the form.
#[derive(Component, Default, Clone)]
pub struct TextureViewBar;

/// The image info (`WxH | format | mips` or `n/a`), in the Properties panel; kept current by the preview.
#[derive(Component, Default, Clone)]
pub struct TextureInfoText;

/// The zoom in the header (`Fit` or a percentage).
#[derive(Component, Default, Clone)]
pub struct TextureZoomText;

/// Settings form, rebuilt on [`TextureDoc::layout`].
#[derive(Component, Default, Clone)]
pub struct TextureViewForm;

/// The preview area (clips the image, takes wheel / drag / double click).
#[derive(Component, Default, Clone)]
pub struct TextureViewArea;

/// The shown image's frame (checkerboard background) and the image's size in pixels; positioned by [`place`].
#[derive(Component)]
pub struct TexturePic(pub UVec2);

/// The path text field (applies on Enter).
#[derive(Component, Default, Clone)]
pub struct TexturePathField;

/// Preview camera of the open texture: `scale` = image px → logical px (`None`: fit the area), `offset` from the
/// area's centre. Reset when another texture is shown.
#[derive(Resource, Default)]
struct PreviewZoom {
    key: Option<String>,
    scale: Option<f32>,
    offset: Vec2,
    /// The scale [`place`] used last (the start of a wheel zoom from "fit").
    shown: f32,
}

pub struct TextureViewPlugin;

impl Plugin for TextureViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PreviewZoom>()
            .add_systems(Update, (redraw_form, redraw_preview, place).chain().after(crate::apply_actions))
            .add_observer(zoom)
            .add_observer(pan)
            .add_observer(fit_on_double_click)
            .add_observer(path_on_enter);
    }
}

pub fn texture_view() -> impl Scene {
    bsn! {
        ViewOf(ViewKind::Texture)
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
        Children [
            TextureViewBar
            Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                align_items: AlignItems::Center,
                column_gap: px(8),
                min_height: px(30),
                padding: UiRect::new(px(10), px(10), px(2), px(2)),
            }
            BackgroundColor(HEADER_BG)
            --
            Node { flex_grow: 1.0, min_height: px(0), flex_direction: FlexDirection::Row }
            Children [
                @scroll_column()
                TextureViewForm
                Node { width: px(FORM_W), flex_grow: 0.0, flex_shrink: 0.0, padding: UiRect::new(px(12), px(12), px(8), px(24)), row_gap: px(4) }
                --
                // The bottom margin keeps the image clear of the file type watermark.
                TextureViewArea
                Node { flex_grow: 1.0, min_width: px(0), margin: UiRect::bottom(px(56)), overflow: Overflow::clip() }
            ]
        ]
    }
}

/// A field's value as text (read-only built-in textures).
fn value_text(v: &Value) -> String {
    match v {
        Value::Bool(b) => if *b { "on" } else { "off" }.to_string(),
        Value::Float(f) => format!("{f}"),
        Value::Int(i) => i.to_string(),
        other => other.as_str().to_string(),
    }
}

/// The widget editing `field` (or its value as text on a read-only document).
fn field_widget(t: &TextureDoc, field: TextureField) -> Box<dyn Scene> {
    let v = field.get(&t.asset.settings);
    if t.is_internal() {
        return Box::new(text(value_text(&v), TEXT, 13.0));
    }
    let target = PropTarget::Texture(field);
    match &v {
        Value::Bool(b) => checkbox_w(target, *b),
        Value::Float(f) => number(target, 0, *f, None),
        Value::Int(i) => integer(target, *i),
        _ => {
            let options: Vec<String> = field.options().into_iter().map(str::to_string).collect();
            let cur = options.iter().position(|o| o == v.as_str()).unwrap_or(0);
            select_w(target, options, Vec::new(), cur)
        }
    }
}

fn hint(s: impl Into<String>) -> Box<dyn Scene> {
    let s = s.into();
    Box::new(bsn! { Node { margin: UiRect::left(px(116)) } Children [ @text(s, TEXT_DIM, 11.0) ] })
}

fn form(t: &TextureDoc, clamp_to_border: bool) -> Vec<Box<dyn Scene>> {
    let mut out: Vec<Box<dyn Scene>> = vec![Box::new(section("Image"))];
    if t.is_internal() {
        out.push(Box::new(labeled("Path".into(), Box::new(text(format!("assets/{}", t.asset.path), TEXT, 13.0)))));
        out.push(hint("Built-in texture: copied there on every regeneration; read-only."));
    } else {
        let init = InitText(t.asset.path.clone());
        let field: Box<dyn Scene> = Box::new(bsn! {
            @FeathersTextInputContainer
            Node { flex_grow: 1.0 }
            Children [ @FeathersTextInput TexturePathField init ]
        });
        out.push(Box::new(labeled("Path".into(), field)));
        out.push(hint("Under the game's assets/ dir (ktx2, png); Enter applies. One file - one Texture asset."));
    }
    // Hints only where something can be changed.
    let editable = !t.is_internal();
    out.push(Box::new(section("Loader")));
    out.push(Box::new(labeled(TextureField::IsSrgb.label().into(), field_widget(t, TextureField::IsSrgb))));
    if editable {
        out.push(hint("Rebuild needed in the game (not hot-reloaded)."));
    }
    let expand = TextureField::ExpandGrayscale;
    out.push(Box::new(labeled(expand.label().into(), field_widget(t, expand))));
    if editable {
        out.push(hint("Rebuild needed in the game. Not previewed: the editor always expands grayscale."));
    }
    out.push(Box::new(section("Sampler")));
    let default = TextureField::DefaultSampler;
    out.push(Box::new(labeled(default.label().into(), field_widget(t, default))));
    if t.asset.settings.sampler.is_none() {
        out.push(hint(
            "The game's ImagePlugin default: with Bevy's ImagePlugin::default() Linear filters, Clamp to edge (not \
             Repeat). Turning it off starts from these values.",
        ));
    } else {
        for f in TextureField::SAMPLER {
            out.push(Box::new(labeled(f.label().into(), field_widget(t, f))));
        }
    }
    for note in previewable(&t.asset.settings, clamp_to_border).1 {
        out.push(Box::new(text(note, WARN, 12.0)));
    }
    out
}

/// Buttons of which one is on (a mode, a preset, a channel), on a shared background.
fn button_group(items: Vec<Box<dyn Scene>>) -> impl Scene {
    bsn! {
        Node { column_gap: px(2), padding: UiRect::all(px(2)), border_radius: BorderRadius::all(px(5)) }
        BackgroundColor(VIEW_BG)
        Children [ {items} ]
    }
}

/// Rebuilds the header and the form when another Texture tab is shown or its form must follow the model.
fn redraw_form(
    editor: Res<Editor>,
    cache: Res<GameImageCache>,
    bar: Query<Entity, With<TextureViewBar>>,
    form_body: Query<Entity, With<TextureViewForm>>,
    mut form_nodes: Query<&mut Node, With<TextureViewForm>>,
    mut last: Local<Option<(u64, bool)>>,
    mut commands: Commands,
) {
    let Some(t) = editor.texture_doc() else { return };
    let key = (t.layout, cache.clamp_to_border);
    if *last == Some(key) {
        return;
    }
    let (Ok(bar), Ok(body)) = (bar.single(), form_body.single()) else { return };
    *last = Some(key);
    commands.entity(bar).despawn_children();
    let mut controls: Vec<Box<dyn Scene>> = Vec::new();
    if !t.is_internal() {
        let modes = [(true, "Easy", "Pick what the texture is for"), (false, "Advanced", "Every setting in a form")]
            .map(|(easy, label, tip)| -> Box<dyn Scene> {
                Box::new(chip(label, Action::Texture(TextureAction::SetEasy(easy)), t.easy == easy, tip, None))
            });
        controls.push(Box::new(button_group(modes.into())));
        if t.easy {
            let current = FilterPreset::of(&t.asset.settings);
            let presets = FilterPreset::ALL.map(|p| -> Box<dyn Scene> {
                Box::new(chip(p.label(), Action::Texture(TextureAction::Preset(p)), current == Some(p), p.tip(), None))
            });
            controls.push(Box::new(bsn! { @text("Use for", TEXT_DIM, 12.0) Node { margin: UiRect::left(px(10)) } }));
            controls.push(Box::new(button_group(presets.into())));
        }
    }
    controls.push(Box::new(bsn! { Node { flex_grow: 1.0 } }));
    let channels =
        [(Some(0), "R"), (Some(1), "G"), (Some(2), "B"), (None, "All")].map(|(c, label)| -> Box<dyn Scene> {
            let tip = match c {
                Some(_) => format!("Preview only the {label} channel, in grey"),
                None => "Preview the image".to_string(),
            };
            Box::new(chip(label, Action::TextureChannel(c), t.channel == c, tip, None))
        });
    controls.push(Box::new(button_group(channels.into())));
    controls.push(Box::new(bsn! { @text("Tile 3x3", TEXT_DIM, 12.0) Node { margin: UiRect::left(px(8)) } }));
    controls.push(checkbox_w(PropTarget::TextureTiled, t.tiled));
    controls.push(Box::new(bsn! { @text("", TEXT_DIM, 12.0) TextureZoomText Node { min_width: px(48) } }));
    for c in controls {
        commands.spawn_scene(bsn! { @{c} ChildOf(bar) });
    }
    // Easy mode has no form: the presets are the whole choice.
    let display = if t.easy && !t.is_internal() { Display::None } else { Display::Flex };
    if let Ok(mut node) = form_nodes.get_mut(body)
        && node.display != display
    {
        node.display = display;
    }
    commands.entity(body).despawn_children();
    for r in form(t, cache.clamp_to_border) {
        commands.spawn_scene(bsn! { @{r} ChildOf(body) });
    }
}

/// What the preview shows: the decoded image (or the error), tiled or not.
#[derive(PartialEq)]
struct Shown {
    root: std::path::PathBuf,
    result: Result<bevy::asset::AssetId<Image>, String>,
    tiled: bool,
    channel: Option<usize>,
}

/// Decodes the image with the current settings (the cache re-checks the file each frame and keeps the decoded
/// image) and rebuilds the preview when the result changed; drops the image once no Texture tab is active.
fn redraw_preview(
    editor: Res<Editor>,
    (mut cache, mut images, checker): (ResMut<GameImageCache>, ResMut<Assets<Image>>, Res<Checker>),
    (area, mut info): (Query<Entity, With<TextureViewArea>>, Query<&mut Text, With<TextureInfoText>>),
    mut zoom: ResMut<PreviewZoom>,
    mut last: Local<Option<Shown>>,
    mut commands: Commands,
) {
    let Ok(area) = area.single() else { return };
    let (Some(t), Some(project)) = (editor.texture_doc(), &editor.project) else {
        if last.take().is_some() {
            commands.entity(area).despawn_children();
        }
        return;
    };
    // Another texture: the camera fits again.
    let key = t.config_key();
    if zoom.key != key {
        *zoom = PreviewZoom { key, ..default() };
    }
    let result = cache.get_channel(project, t.image_key(), &t.asset.settings, t.channel, &mut images);
    let shown = Shown {
        root: project.root.clone(),
        result: result.as_ref().map(|(h, _)| h.id()).map_err(Clone::clone),
        tiled: t.tiled,
        channel: t.channel,
    };
    let line = match &result {
        Ok((h, _)) => images.get(h).map_or_else(|| "n/a".to_string(), describe),
        Err(_) => "n/a".to_string(),
    };
    for mut text in &mut info {
        if text.0 != line {
            text.0 = line.clone();
        }
    }
    if last.as_ref() == Some(&shown) {
        return;
    }
    *last = Some(shown);
    commands.entity(area).despawn_children();
    match result {
        Ok((handle, grey)) => {
            let Some(size) = images.get(&handle).map(Image::size) else { return };
            let tiled = NodeImageMode::Tiled { tile_x: true, tile_y: true, stretch_value: 6.0 };
            let frame = commands
                .spawn((
                    ImageNode::new(checker.0.clone()).with_mode(tiled),
                    TexturePic(size),
                    Node { position_type: PositionType::Absolute, ..default() },
                    Pickable::IGNORE,
                    ChildOf(area),
                ))
                .id();
            // A channel the cache could not turn grey (compressed formats) is shown tinted instead.
            let tint = match t.channel {
                Some(c) if !grey => Color::srgb((c == 0) as u8 as f32, (c == 1) as u8 as f32, (c == 2) as u8 as f32),
                _ => Color::WHITE,
            };
            let mut image = ImageNode::new(handle).with_color(tint);
            if t.tiled {
                // UVs -1..2: the sampler's address modes decide what lies around the image.
                let s = size.as_vec2();
                image.rect = Some(Rect::from_corners(-s, 2.0 * s));
            }
            commands.spawn((
                image,
                Node { width: percent(100), height: percent(100), ..default() },
                Pickable::IGNORE,
                ChildOf(frame),
            ));
        }
        Err(e) => {
            commands.spawn_scene(bsn! { @text(e, ERROR, 14.0) Node { margin: UiRect::all(px(PAD)) } ChildOf(area) });
        }
    }
}

/// Image size shown with tiling (3x3 when tiled).
fn content_px(pic: &TexturePic, tiled: bool) -> Vec2 {
    pic.0.as_vec2().max(Vec2::ONE) * if tiled { 3.0 } else { 1.0 }
}

/// The scale that fits `content` into the area (small images are enlarged).
fn fit_scale(area: Vec2, content: Vec2) -> f32 {
    let room = (area - Vec2::splat(2.0 * PAD)).max(Vec2::ONE);
    (room / content).min_element()
}

/// Positions the image in the area: fitted, or at the zoom's scale and offset from the centre.
fn place(
    editor: Res<Editor>,
    area: Query<&ComputedNode, With<TextureViewArea>>,
    mut pics: Query<(&TexturePic, &mut Node), Without<TextureViewArea>>,
    mut zoom_text: Query<&mut Text, With<TextureZoomText>>,
    mut zoom: ResMut<PreviewZoom>,
) {
    let (Ok(area), Some(t)) = (area.single(), editor.texture_doc()) else { return };
    let size = area.size() * area.inverse_scale_factor();
    for (pic, mut node) in &mut pics {
        let content = content_px(pic, t.tiled);
        let scale = zoom.scale.unwrap_or_else(|| fit_scale(size, content));
        zoom.shown = scale;
        let shown = content * scale;
        let pos = (size - shown) / 2.0 + zoom.offset;
        let (w, h, l, top) = (px(shown.x.round()), px(shown.y.round()), px(pos.x.round()), px(pos.y.round()));
        if node.width != w || node.height != h || node.left != l || node.top != top {
            node.width = w;
            node.height = h;
            node.left = l;
            node.top = top;
        }
    }
    let label = match zoom.scale {
        None => "Fit".to_string(),
        Some(s) => format!("{:.0}%", s * 100.0),
    };
    for mut text in &mut zoom_text {
        if text.0 != label {
            text.0 = label.clone();
        }
    }
}

/// The wheel zooms around the pointer (one notch: x1.25).
fn zoom(
    mut ev: On<PointerScroll>,
    areas: Query<(&ComputedNode, &UiGlobalTransform), With<TextureViewArea>>,
    ratio: Res<MouseScrollPixelsPerLine>,
    mut zoom: ResMut<PreviewZoom>,
) {
    let Ok((node, transform)) = areas.get(ev.entity) else { return };
    ev.propagate(false);
    let notches = ev.to_lines(&ratio).y;
    if notches == 0.0 {
        return;
    }
    let from = zoom.scale.unwrap_or(zoom.shown).max(ZOOM_MIN);
    let to = (from * 1.25f32.powf(notches)).clamp(ZOOM_MIN, ZOOM_MAX);
    // The image point under the pointer stays there: offset' = p - (p - offset) * to / from (from the centre).
    let center = transform.translation * node.inverse_scale_factor();
    let p = ev.pointer.position - center;
    zoom.offset = p - (p - zoom.offset) * (to / from);
    zoom.scale = Some(to);
}

fn pan(mut ev: On<PointerDrag>, areas: Query<(), With<TextureViewArea>>, mut zoom: ResMut<PreviewZoom>) {
    if !areas.contains(ev.entity) {
        return;
    }
    ev.propagate(false);
    if zoom.scale.is_none() {
        zoom.scale = Some(zoom.shown);
    }
    zoom.offset += ev.delta;
}

fn fit_on_double_click(ev: On<PointerClick>, areas: Query<(), With<TextureViewArea>>, mut zoom: ResMut<PreviewZoom>) {
    if areas.contains(ev.entity) && ev.button == PointerButton::Primary && ev.count >= 2 {
        zoom.scale = None;
        zoom.offset = Vec2::ZERO;
    }
}

/// Enter in the path field points the asset at the typed file.
fn path_on_enter(
    ev: On<FocusedInput<KeyboardInput>>,
    fields: Query<&EditableText, With<TexturePathField>>,
    mut actions: ResMut<Actions>,
) {
    if ev.input.key_code == KeyCode::Enter
        && ev.input.state.is_pressed()
        && let Ok(t) = fields.get(ev.focused_entity)
    {
        actions.push(Action::Texture(TextureAction::SetPath(t.value().to_string())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_scale_keeps_aspect_and_enlarges() {
        let area = Vec2::new(400.0, 300.0) + Vec2::splat(2.0 * PAD);
        assert_eq!(fit_scale(area, Vec2::new(100.0, 100.0)), 3.0);
        assert_eq!(fit_scale(area, Vec2::new(800.0, 300.0)), 0.5);
    }

    #[test]
    fn value_text_reads_fields() {
        assert_eq!(value_text(&Value::Bool(true)), "on");
        assert_eq!(value_text(&Value::Int(4)), "4");
        assert_eq!(value_text(&Value::Str("Linear".into())), "Linear");
    }
}
