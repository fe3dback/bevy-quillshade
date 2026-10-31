//! The `Light Transition Preview` tool (`Tools -> Light Transition Preview`, design §9.2): lets the user pick `Env A`,
//! `Env B`, and a transition `Curve` (`Linear` or any `.curve.ron` in `quillshade/light/`), scrub progress `t` in
//! `0.0..=1.0`, and evaluate the interpolated environment live in a 3D Render-to-Texture viewport.

use std::collections::BTreeSet;

use bevy::prelude::*;
use curve_kit::editor::CurveDoc;
use editor_core::action::{Handled, TabAction};
use editor_core::doc_kind::{DocCtx, StatusCtx, StatusInfo, TabDoc};
use editor_core::domain::ToolId;
use editor_core::dynval::{PropKey, PropTarget, PropValue};
use editor_core::editors::{EditorAreas, SidebarKey, ViewOf, bar_row, fill_sidebar_with, sidebar_body};
use editor_core::preview::{PreviewHandle, start_preview};
use editor_core::state::editor::Editor;
use editor_core::tab::ViewKindId;
use editor_core::theme::{TEXT, TEXT_DIM, text};
use editor_core::ui::icons::IconRef;
use editor_core::ui::props::{hotkey_row, number_ranged, select_values};
use editor_core::ui::shell::VIEW_BG;
use editor_core::ui::widgets::subpane_box;
use project_core::{AssetRef, Project};

use super::preview::{LIGHT_PREVIEW_KIND, interpolate_environments};
use super::{EnvironmentDoc, LightCurve, LightState};
use crate::backend::{CURVE_SUFFIX, ENVIRONMENT_SUFFIX, Environment, FogFalloffDef, LIGHT};

/// The `Light Preview` tool's id in the Light domain.
pub const LIGHT_PREVIEW_TOOL: ToolId = "light_preview";

/// The `Light Preview` tool tab's view.
pub const LIGHT_PREVIEW_VIEW: ViewKindId = ViewKindId("light.preview_tool");

/// A control of the `Light Preview` tool topbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightPreviewProp {
    EnvA,
    EnvB,
    Curve,
    T,
}

impl PropTarget for LightPreviewProp {}

/// The `Light Preview` tool tab's document.
pub struct LightPreviewDoc {
    /// Source environment path (`quillshade/light/<env_a>.environment.ron`).
    pub env_a: String,
    /// Target environment path (`quillshade/light/<env_b>.environment.ron`).
    pub env_b: String,
    /// Transition curve path (`""` for built-in `"Linear"`, else `quillshade/light/<curve>.curve.ron`).
    pub curve: String,
    /// Normalized transition progress `t` in `0.0..=1.0`.
    pub t: f32,
    /// RAII 3D preview studio session handle (`editor_core::preview::start_preview`).
    pub preview: PreviewHandle,
    /// Bumped whenever any preview parameter changes.
    pub revision: u64,
    /// Bumped when the topbar controls must be rebuilt (`EnvA`, `EnvB`, `Curve`, or a clamped `T`).
    pub layout: u64,
}

impl LightPreviewDoc {
    pub fn new(env_a: String, env_b: String, curve: String, t: f32) -> Self {
        Self {
            env_a,
            env_b,
            curve,
            t: t.clamp(0.0, 1.0),
            preview: start_preview(LIGHT_PREVIEW_KIND),
            revision: 1,
            layout: 1,
        }
    }
}

/// Opens the `Light Preview` tool tab, defaulting `env_a` to the first environment and `env_b` to the second (or
/// `env_a` if only one exists).
pub(super) fn open(editor: &mut Editor) -> Box<dyn TabDoc> {
    let envs = light_environments(editor);
    let env_a = envs.first().cloned().unwrap_or_default();
    let env_b = envs.get(1).or_else(|| envs.first()).cloned().unwrap_or_default();
    Box::new(LightPreviewDoc::new(env_a, env_b, String::new(), 0.0))
}

impl TabDoc for LightPreviewDoc {
    fn apply(&mut self, ctx: &mut DocCtx, action: &TabAction) -> Handled {
        match action {
            TabAction::SetValue { key, value, .. } => {
                let Some(prop) = key.downcast::<LightPreviewProp>() else { return Handled::No };
                match prop {
                    LightPreviewProp::EnvA => {
                        self.env_a = value.as_str().to_string();
                        self.revision += 1;
                        self.layout += 1;
                        ctx.redraw_props = true;
                    }
                    LightPreviewProp::EnvB => {
                        self.env_b = value.as_str().to_string();
                        self.revision += 1;
                        self.layout += 1;
                        ctx.redraw_props = true;
                    }
                    LightPreviewProp::Curve => {
                        self.curve = value.as_str().to_string();
                        self.revision += 1;
                        self.layout += 1;
                        ctx.redraw_props = true;
                    }
                    LightPreviewProp::T => {
                        self.t = value.as_f32().clamp(0.0, 1.0);
                        self.revision += 1;
                        self.layout += 1;
                        ctx.redraw_props = true;
                    }
                }
            }
            TabAction::SetComponent { key, value, is_final, .. } => {
                let Some(prop) = key.downcast::<LightPreviewProp>() else { return Handled::No };
                if *prop == LightPreviewProp::T {
                    let clamped = value.clamp(0.0, 1.0);
                    self.t = clamped;
                    self.revision += 1;
                    if *is_final && clamped != *value {
                        self.layout += 1;
                    }
                    ctx.redraw_props = *is_final;
                }
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn current_value(&self, key: &PropKey, _ctx: &StatusCtx) -> Option<PropValue> {
        match *key.downcast::<LightPreviewProp>()? {
            LightPreviewProp::EnvA => Some(PropValue::Str(self.env_a.clone())),
            LightPreviewProp::EnvB => Some(PropValue::Str(self.env_b.clone())),
            LightPreviewProp::Curve => Some(PropValue::Str(self.curve.clone())),
            LightPreviewProp::T => Some(PropValue::Float(self.t)),
        }
    }

    fn asset(&self) -> Option<&AssetRef> {
        None
    }

    fn view(&self) -> ViewKindId {
        LIGHT_PREVIEW_VIEW
    }

    fn base_title(&self) -> String {
        "Light Transition Preview".into()
    }

    fn icon(&self) -> IconRef {
        super::LIGHT_ICON
    }

    fn watermark(&self) -> Option<&'static str> {
        Some("Light Transition Preview")
    }

    fn is_dirty(&self) -> bool {
        false
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn save(&mut self, _ctx: &mut DocCtx) -> std::io::Result<()> {
        Ok(())
    }

    fn changed_on_disk(&self, _project: &Project) -> bool {
        false
    }

    fn reload_from_disk(&mut self, _ctx: &mut DocCtx) -> Result<(), String> {
        Ok(())
    }

    fn keep_mine(&mut self, _project: &Project) {}

    fn rename(&mut self, _project: &Project, from: &AssetRef, to: &AssetRef) {
        if from.domain != LIGHT {
            return;
        }
        let mut changed = false;
        if from.suffix == ENVIRONMENT_SUFFIX {
            if self.env_a == from.path {
                self.env_a = to.path.clone();
                changed = true;
            }
            if self.env_b == from.path {
                self.env_b = to.path.clone();
                changed = true;
            }
        } else if from.suffix == CURVE_SUFFIX && self.curve == from.path {
            self.curve = to.path.clone();
            changed = true;
        }
        if changed {
            self.revision += 1;
            self.layout += 1;
        }
    }

    fn config_key(&self) -> Option<String> {
        None
    }

    fn tool(&self) -> Option<ToolId> {
        Some(LIGHT_PREVIEW_TOOL)
    }

    fn status(&self, ctx: &StatusCtx) -> Option<StatusInfo> {
        Some(StatusInfo::new(false, ctx, vec![format!("t = {:.2}", self.t)]))
    }

    fn preview_handle(&self) -> Option<&PreviewHandle> {
        Some(&self.preview)
    }
}

/// The `Light Preview` editor kind.
pub struct LightPreviewEditor;

/// The editor topbar row of the `Light Preview` tab.
#[derive(Component, Default, Clone)]
pub struct LightPreviewBar;

/// The right sidebar body of the `Light Preview` tab.
#[derive(Component, Default, Clone)]
pub struct LightPreviewSidebarBody;

/// Live curve-sampled progress readout (`s = {:.3}`) in the `Light Preview` topbar.
#[derive(Component, Default, Clone)]
pub struct LightPreviewSampleText;

impl EditorAreas for LightPreviewEditor {
    const VIEW: ViewKindId = LIGHT_PREVIEW_VIEW;
    const SIDEBAR_TITLE: &'static str = "Light Transition Preview";

    fn topbar() -> Option<impl Scene> {
        Some(bsn! { @bar_row() LightPreviewBar })
    }

    fn sidebar() -> Option<impl Scene> {
        Some(bsn! { @sidebar_body() LightPreviewSidebarBody })
    }

    fn build(app: &mut App) {
        app.add_systems(Update, (redraw_bar, update_sample_text, redraw_sidebar).after(editor_core::apply_actions));
    }
}

fn active(editor: &Editor) -> Option<(usize, &LightPreviewDoc)> {
    let i = editor.active?;
    Some((i, editor.tabs.get(i)?.get::<LightPreviewDoc>()?))
}

/// The `Light Preview` view in the editor content (full-area 3D RTT viewport).
pub fn light_preview_view() -> impl Scene {
    let area: Box<dyn Scene> = Box::new(super::preview::preview_viewport(LIGHT_PREVIEW_VIEW));
    bsn! {
        ViewOf(LIGHT_PREVIEW_VIEW)
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
            @{area}
        ]
    }
}

pub(super) fn light_environments(editor: &Editor) -> Vec<String> {
    let mut set = BTreeSet::new();
    if let Some(state) = editor.domains.get::<LightState>() {
        set.extend(state.assets.environments.keys().cloned());
    }
    if let Some(files) = editor.kind_files.get(&LIGHT) {
        for r in files.iter().filter(|r| r.suffix == ENVIRONMENT_SUFFIX) {
            set.insert(r.path.clone());
        }
    }
    for t in editor.tabs.iter().filter(|t| t.domain == LIGHT) {
        if let Some(d) = t.get::<EnvironmentDoc>() {
            set.insert(d.aref.path.clone());
        }
    }
    set.into_iter().collect()
}

pub(super) fn light_curves(editor: &Editor) -> Vec<String> {
    let mut set = BTreeSet::new();
    if let Some(state) = editor.domains.get::<LightState>() {
        set.extend(state.assets.curves.keys().cloned());
    }
    if let Some(files) = editor.kind_files.get(&LIGHT) {
        for r in files.iter().filter(|r| r.suffix == CURVE_SUFFIX) {
            set.insert(r.path.clone());
        }
    }
    for t in editor.tabs.iter().filter(|t| t.domain == LIGHT) {
        if let Some(d) = t.get::<CurveDoc<LightCurve>>() {
            set.insert(d.aref.path.clone());
        }
    }
    set.into_iter().collect()
}

pub(super) fn env_options(envs: &[String], current: &str) -> (Vec<String>, Vec<String>, usize) {
    let mut labels = envs.to_vec();
    let mut values = envs.to_vec();
    if !current.is_empty() && !values.iter().any(|v| v == current) {
        labels.push(format!("{current} (missing)"));
        values.push(current.to_string());
    }
    if values.is_empty() {
        labels.push("None".to_string());
        values.push(String::new());
    }
    let at = values.iter().position(|v| v == current).unwrap_or(0);
    (labels, values, at)
}

pub(super) fn curve_options(curves: &[String], current: &str) -> (Vec<String>, Vec<String>, usize) {
    let (mut labels, mut values) = (vec!["Linear".to_string()], vec![String::new()]);
    for c in curves {
        labels.push(c.clone());
        values.push(c.clone());
    }
    if !current.is_empty() && !curves.iter().any(|c| c == current) {
        labels.push(format!("{current} (missing)"));
        values.push(current.to_string());
    }
    let at = values.iter().position(|v| v == current).unwrap_or(0);
    (labels, values, at)
}

fn resolve_preview(editor: &Editor, doc: &LightPreviewDoc) -> (Environment, f32) {
    let shown =
        editor.domains.get::<LightState>().map(|s| super::state::shown(&s.assets, &editor.tabs)).unwrap_or_default();
    let from = shown.environments.get(&doc.env_a).cloned().unwrap_or_default();
    let to = shown.environments.get(&doc.env_b).cloned().unwrap_or_default();
    let raw_t = doc.t.clamp(0.0, 1.0);
    let s = if doc.curve.is_empty() {
        raw_t
    } else {
        shown.curves.get(&doc.curve).map_or(raw_t, |c| c.sample(raw_t, curve_kit::Interp::CatmullRom))
    };
    let env = interpolate_environments(&from, &to, raw_t, s);
    (env, s)
}

type BarKey = (u64, usize, u64, Vec<String>, Vec<String>);

fn redraw_bar(
    editor: Res<Editor>,
    bar: Query<Entity, With<LightPreviewBar>>,
    mut last: Local<Option<BarKey>>,
    mut commands: Commands,
) {
    let Some((i, doc)) = active(&editor) else {
        *last = None;
        return;
    };
    let envs = light_environments(&editor);
    let curves = light_curves(&editor);
    let key = (editor.project_generation, i, doc.layout, envs, curves);
    if last.as_ref() == Some(&key) {
        return;
    }
    let Ok(bar) = bar.single() else { return };
    commands.entity(bar).despawn_children();

    let (env_a_labels, env_a_values, env_a_idx) = env_options(&key.3, &doc.env_a);
    let (env_b_labels, env_b_values, env_b_idx) = env_options(&key.3, &doc.env_b);
    let (curve_labels, curve_values, curve_idx) = curve_options(&key.4, &doc.curve);

    let env_a_select = select_values(LightPreviewProp::EnvA, env_a_labels, env_a_values, env_a_idx);
    let env_b_select = select_values(LightPreviewProp::EnvB, env_b_labels, env_b_values, env_b_idx);
    let curve_select = select_values(LightPreviewProp::Curve, curve_labels, curve_values, curve_idx);
    let t_input = number_ranged(LightPreviewProp::T, 0, doc.t, Some((0.0, 1.0)), Some((0.0, 1.0)));
    let (_, s) = resolve_preview(&editor, doc);
    let sample_str = format!("s = {s:.3}");

    commands.spawn_scene(bsn! {
        @text("Env A", TEXT_DIM, 12.0)
        ChildOf(bar)
    });
    commands.spawn_scene(bsn! {
        Node { min_width: px(96) }
        Children [ @{env_a_select} ]
        ChildOf(bar)
    });
    commands.spawn_scene(bsn! {
        @text("Env B", TEXT_DIM, 12.0)
        ChildOf(bar)
    });
    commands.spawn_scene(bsn! {
        Node { min_width: px(96) }
        Children [ @{env_b_select} ]
        ChildOf(bar)
    });
    commands.spawn_scene(bsn! {
        @text("Curve", TEXT_DIM, 12.0)
        ChildOf(bar)
    });
    commands.spawn_scene(bsn! {
        Node { min_width: px(96) }
        Children [ @{curve_select} ]
        ChildOf(bar)
    });
    commands.spawn_scene(bsn! {
        @text("t", TEXT_DIM, 12.0)
        ChildOf(bar)
    });
    commands.spawn_scene(bsn! {
        Node { width: px(88) }
        Children [ @{t_input} ]
        ChildOf(bar)
    });
    commands.spawn_scene(bsn! {
        LightPreviewSampleText
        @text(sample_str, TEXT_DIM, 12.0)
        ChildOf(bar)
    });

    *last = Some(key);
}

fn update_sample_text(editor: Res<Editor>, mut q: Query<&mut Text, With<LightPreviewSampleText>>) {
    let Some((_, doc)) = active(&editor) else { return };
    let (_, s) = resolve_preview(&editor, doc);
    let want = format!("s = {s:.3}");
    for mut t in &mut q {
        if t.0 != want {
            t.0 = want.clone();
        }
    }
}

fn redraw_sidebar(
    editor: Res<Editor>,
    body: Query<Entity, With<LightPreviewSidebarBody>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some((_, doc)) = active(&editor) else { return key.forget() };
    fill_sidebar_with(&mut commands, &body, &mut key, SidebarKey::now(&editor), || sidebar_rows(&editor, doc));
}

const HOTKEYS: [(&str, &str); 3] =
    [("Drag", "Orbit preview camera"), ("Wheel", "Zoom preview camera"), ("Double-click", "Reset camera")];

fn sidebar_rows(editor: &Editor, doc: &LightPreviewDoc) -> Vec<Box<dyn Scene>> {
    let (env, s) = resolve_preview(editor, doc);
    let env_a_label = if doc.env_a.is_empty() { "None" } else { doc.env_a.as_str() };
    let env_b_label = if doc.env_b.is_empty() { "None" } else { doc.env_b.as_str() };
    let curve_label = if doc.curve.is_empty() { "Linear" } else { doc.curve.as_str() };
    let fog_summary = if !env.fog.enabled {
        "Disabled".to_string()
    } else {
        match &env.fog.falloff {
            FogFalloffDef::Linear { start, end } => format!("Linear ({start:.1}..{end:.1})"),
            FogFalloffDef::Exponential { density } => format!("Exponential (density {density:.4})"),
            FogFalloffDef::ExponentialSquared { density } => format!("ExponentialSquared (density {density:.4})"),
            FogFalloffDef::Atmospheric { .. } => "Atmospheric".to_string(),
        }
    };

    let info: Vec<Box<dyn Scene>> = vec![
        Box::new(text(format!("{env_a_label} -> {env_b_label}"), TEXT, 14.0)),
        Box::new(text(format!("Curve: {curve_label}"), TEXT_DIM, 12.0)),
        Box::new(text(format!("Progress: t = {:.2}, s = {s:.3}", doc.t), TEXT_DIM, 12.0)),
        Box::new(text(format!("Exposure (EV100): {:.2}", env.ev100), TEXT_DIM, 12.0)),
        Box::new(text(format!("Tonemapping: {}", env.tonemapping.label()), TEXT_DIM, 12.0)),
        Box::new(text(format!("Ambient brightness: {:.1}", env.ambient_brightness), TEXT_DIM, 12.0)),
        Box::new(text(format!("Fog: {fog_summary}"), TEXT_DIM, 12.0)),
        Box::new(text(format!("Directional lights: {}", env.lights.len()), TEXT_DIM, 12.0)),
    ];
    let hotkeys = HOTKEYS.map(|(keys, what)| -> Box<dyn Scene> { Box::new(hotkey_row(keys, what)) });
    vec![subpane_box("Transition Preview", info), subpane_box("Hotkeys", hotkeys.into())]
}
