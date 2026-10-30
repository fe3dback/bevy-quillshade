//! Gradients editor kind: a gradient collection (gradient design §8, [`GradientsDoc`]), layout 2 of the approved
//! mockup.
//!
//! - Editor topbar ([`GradientsViewBar`]): the file path and Duplicate / Clear / Move up / Move down of the selected
//!   slot.
//! - Right sidebar ([`GradientsSidebarBody`]): the selected slot's Name, the list of all [`SLOTS`] slots (a
//!   `FeathersListView` whose rows show a [`GradientStrip`], persistent so its scroll offset stays; `sync_slot_list`
//!   keeps the rows on the open collection), the hotkeys.
//! - Editor content: `ui::gradient_view`.
//! - Hotkeys ([`GradientsCtx`]): the shared edit keys (routed to the collection by `Editor`) and Left / Right nudging
//!   the selected stop.

use bevy::feathers::controls::FeathersListView;
use bevy::prelude::*;
use bevy::ui::Selected;
use bevy::ui_widgets::ValueChange;
use bevy_enhanced_input::prelude::{Actions as InputActions, InputAction, ModKeys, Start};
use editor_core::action::{Handled, TabAction};
use editor_core::dynval::{PropKey, PropValue};
use editor_core::tab::ViewKindId;
use project_core::AssetRef;
use shader_graph_core::gradient::{GradientCollection, SAMPLES, SLOTS, Slot};
use shader_project::{AssetId, Project, assets, ops};

use super::main_input::{EditCopy, EditDelete, EditDuplicate, EditPaste, add_kind, ctrl, hotkey, plain};
use super::{EditorAreas, EditorInput, SidebarKey, StatusCtx, StatusInfo, fill_sidebar_with};
use crate::doc_kind::{DocCtx, TabDoc, asset_title, io_error, project_of};
use crate::gradients_doc::{GradientSel, GradientsDoc};
use crate::keys::{mods, no_command_mods};
use crate::ops::prelude::*;
use crate::ops::{Action, Actions, Editor, GradientProp, GradientsAction};
use crate::ui::gradient_strip::{GradientStrip, slot_list_row};
use crate::ui::icons::{IconId, IconRef};
use crate::ui::props::{hotkey_row, labeled, text_hint};
use crate::ui::widgets::{Caption, GroupButton, button_group, subpane_box};
use crate::ui::{TEXT, TEXT_DIM, clipped_text, text};
use crate::util::text::plural;

/// The gradients editor's window areas.
pub struct GradientsEditor;

/// Gradient collection: the selected slot's stops + the list of slots (gradient design §8).
pub const GRADIENTS: ViewKindId = ViewKindId("material.gradients");

impl EditorAreas for GradientsEditor {
    const VIEW: ViewKindId = GRADIENTS;
    const SIDEBAR_TITLE: &'static str = "Gradients";

    /// The bar row, filled by [`redraw_bar`].
    fn topbar() -> Option<impl Scene> {
        Some(bsn! { @super::bar_row() GradientsViewBar })
    }

    /// The selected slot's name, the slot list and the hotkeys. The column fills the right sidebar, so the list
    /// scrolls on its own (with its Feathers scrollbar) and the hotkeys stay under it.
    fn sidebar() -> Option<impl Scene> {
        let rows: Vec<Box<dyn SceneList>> = (0..SLOTS).map(slot_row).collect();
        let rows: Box<dyn SceneList> = Box::new(rows);
        let hotkeys = GRADIENT_HOTKEYS.map(|(keys, what)| -> Box<dyn Scene> { Box::new(hotkey_row(keys, what)) });
        let hotkeys = subpane_box("Hotkeys", hotkeys.into());
        Some(bsn! {
            GradientsSidebarBody
            Node { flex_direction: FlexDirection::Column, row_gap: px(6), flex_grow: 1.0, min_height: px(0) }
            Children [
                GradientNameBox
                Node { flex_direction: FlexDirection::Column, flex_shrink: 0.0 }
                --
                @FeathersListView { @rows: rows }
                SlotList
                Node { flex_grow: 1.0, flex_basis: px(0), min_height: px(160) }
                --
                Node { flex_shrink: 0.0, flex_direction: FlexDirection::Column }
                Children [ @{hotkeys} ]
            ]
        })
    }

    fn build(app: &mut App) {
        app.add_systems(Update, (redraw_bar, redraw_name, sync_slot_list).after(crate::apply_actions))
            .add_observer(select_slot_row);
    }

    fn input(app: &mut App) {
        add_kind::<Self>(app);
        app.add_observer(nudge::<-1>).add_observer(nudge::<1>);
    }
}

/// The gradients editor's input context: the edit keys (routed to the collection by `Editor`) and arrow nudging.
#[derive(Component, Default, Clone)]
pub struct GradientsCtx;

impl EditorInput for GradientsEditor {
    type Ctx = GradientsCtx;

    fn bindings() -> impl Scene {
        use KeyCode::{ArrowLeft, ArrowRight, Backspace, Delete, KeyC, KeyD, KeyV};
        bsn! {
            InputActions<GradientsCtx> [
                @{hotkey::<EditDuplicate>(ctrl(KeyD))}
                --
                @{hotkey::<EditCopy>(ctrl(KeyC))}
                --
                @{hotkey::<EditPaste>(ctrl(KeyV))}
                --
                @{hotkey::<EditDelete>(plain([Delete, Backspace]))}
                --
                @{hotkey::<StopNudge<-1>>(plain([ArrowLeft]))}
                --
                @{hotkey::<StopNudge<1>>(plain([ArrowRight]))}
            ]
        }
    }
}

/// Arrow-key step of a stop's `t` (Shift: ten times).
const NUDGE: f32 = 0.01;

/// Left (`-1`) / Right (`1`): moves the selected stop.
pub struct StopNudge<const DIR: i8>;

impl<const DIR: i8> InputAction for StopNudge<DIR> {
    type Output = bool;
}

/// Arrow keys move the selected stop along `t` (Shift: ten times further), one undo step each.
fn nudge<const DIR: i8>(
    _: On<Start<StopNudge<DIR>>>,
    keys: Res<ButtonInput<KeyCode>>,
    editor: Res<Editor>,
    mut actions: ResMut<Actions>,
) {
    let Some(g) = editor.gradients_doc() else { return };
    let Some(GradientSel::Stop(i)) = g.selection else { return };
    let Some(stop) = g.current().and_then(|s| s.stops.get(i)) else { return };
    if !no_command_mods(&keys) {
        return;
    }
    let step = if mods(&keys).contains(ModKeys::SHIFT) { NUDGE * 10.0 } else { NUDGE };
    actions.push(Action::from(GradientsAction::SetStopT(i, stop.t + f32::from(DIR) * step, None)));
}

/// The gradients editor topbar row (path, slot buttons), rebuilt with the tab, the slot and what the buttons can do.
#[derive(Component, Default, Clone)]
pub struct GradientsViewBar;

/// The gradients editor's right sidebar body.
#[derive(Component, Default, Clone)]
pub struct GradientsSidebarBody;

/// The selected slot's subpane (its Name), refilled on the right sidebar's [`SidebarKey`].
#[derive(Component, Default, Clone)]
pub struct GradientNameBox;

/// The slot list (`FeathersListView`).
#[derive(Component, Default, Clone)]
pub struct SlotList;

/// A row of the slot list: the slot at this position.
#[derive(Component, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotRow(pub usize);

/// The strip of the slot list row at this position.
#[derive(Component, Default, Clone, Copy)]
struct SlotStrip(usize);

/// The name label over the strip of the slot list row at this position.
#[derive(Component, Default, Clone, Copy)]
struct SlotName(usize);

/// One slot list row: its position, the gradient strip and its name over the strip's right end. Empty slots show
/// the strip's hatching.
fn slot_row(pos: usize) -> Box<dyn SceneList> {
    let (row, strip, name) = (SlotRow(pos), SlotStrip(pos), SlotName(pos));
    slot_list_row(
        pos.to_string(),
        Box::new(bsn! { row }),
        Box::new(bsn! { strip GradientStrip ImageNode }),
        Box::new(bsn! { name Text("") }),
    )
}

/// Keeps the slot list on the active collection: each row's strip shows the slot at its position, its name label
/// the slot's name, and the selected slot's row is `Selected` (the model is the source of truth; Feathers styles the
/// row). Runs when the tab, its document or the rows change.
#[allow(clippy::type_complexity)]
fn sync_slot_list(
    editor: Res<Editor>,
    rows: Query<(Entity, &SlotRow, Has<Selected>)>,
    added: Query<(), Added<SlotRow>>,
    mut strips: Query<(&SlotStrip, &mut GradientStrip)>,
    mut names: Query<(&SlotName, &mut Text)>,
    mut last: Local<Option<(Option<usize>, String, u64)>>,
    mut commands: Commands,
) {
    let Some(g) = editor.gradients_doc() else { return };
    let key = (editor.active, g.id.path.clone(), g.revision);
    if last.as_ref() == Some(&key) && added.is_empty() {
        return;
    }
    *last = Some(key);
    for (e, row, selected) in &rows {
        match (row.0 == g.slot, selected) {
            (true, false) => _ = commands.entity(e).try_insert(Selected),
            (false, true) => _ = commands.entity(e).try_remove::<Selected>(),
            _ => {}
        }
    }
    for (pos, mut strip) in &mut strips {
        let Some(slot) = g.collection.slots.get(pos.0) else { continue };
        if strip.collection != g.id.path || strip.slot != slot.id {
            *strip = GradientStrip { collection: g.id.path.clone(), slot: slot.id };
        }
    }
    for (pos, mut name) in &mut names {
        let want = g.collection.slots.get(pos.0).map_or("", |s| s.name.as_str());
        if name.0 != want {
            name.0 = want.to_string();
        }
    }
}

/// A row picked in the slot list (click, or Space / Enter on the list's active row) selects its slot.
fn select_slot_row(
    ev: On<ValueChange<Entity>>,
    lists: Query<(), With<SlotList>>,
    rows: Query<&SlotRow>,
    mut actions: ResMut<Actions>,
) {
    if lists.contains(ev.source)
        && let Ok(row) = rows.get(ev.value)
    {
        actions.push(Action::from(GradientsAction::SelectSlot(row.0)));
    }
}

fn redraw_bar(
    editor: Res<Editor>,
    bar: Query<Entity, With<GradientsViewBar>>,
    mut last: Local<Option<(String, usize, bool, bool)>>,
    mut commands: Commands,
) {
    let Some(g) = editor.gradients_doc() else { return };
    let Some(slot) = g.current() else { return };
    let key = (g.id.path.clone(), g.slot, slot.is_empty(), slot.name.is_empty());
    if last.as_ref() == Some(&key) {
        return;
    }
    let Ok(bar) = bar.single() else { return };
    *last = Some(key);
    commands.entity(bar).despawn_children();
    commands.spawn_scene(bsn! { @clipped_text(g.id.source_rel(), TEXT, 13.0) ChildOf(bar) });
    commands.spawn_scene(bsn! { Node { flex_grow: 1.0 } ChildOf(bar) });
    commands.spawn_scene(bsn! { @text(format!("Slot {}", g.slot), TEXT_DIM, 12.0) ChildOf(bar) });
    let buttons = slot_buttons(g.slot, slot);
    commands.spawn_scene(bsn! { @button_group(buttons) ChildOf(bar) });
}

/// Duplicate / Clear / Move up / Move down of the slot at `pos`, disabled where they would do nothing.
fn slot_buttons(pos: usize, slot: &Slot) -> Vec<GroupButton> {
    let button = |icon: IconId, action, disabled, tip: &str| GroupButton {
        caption: Caption::Icon(icon.into()),
        action: Action::from(action),
        selected: false,
        disabled,
        tip: tip.into(),
    };
    let empty = slot.is_empty();
    vec![
        button(IconId::Copy, GradientsAction::DuplicateSlot, empty, "Duplicate gradient into the next empty slot"),
        button(IconId::Clear, GradientsAction::ClearSlot, empty && slot.name.is_empty(), "Clear gradient"),
        button(IconId::ArrowUp, GradientsAction::MoveSlot(pos, pos.saturating_sub(1)), pos == 0, "Move gradient up"),
        button(IconId::ArrowDown, GradientsAction::MoveSlot(pos, pos + 1), pos + 1 >= SLOTS, "Move gradient down"),
    ]
}

fn redraw_name(
    editor: Res<Editor>,
    body: Query<Entity, With<GradientNameBox>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some(g) = editor.gradients_doc() else { return key.forget() };
    fill_sidebar_with(&mut commands, &body, &mut key, SidebarKey::now(&editor), || name_rows(g));
}

/// The selected slot's subpane: its editor-only name and what it holds.
fn name_rows(g: &GradientsDoc) -> Vec<Box<dyn Scene>> {
    let Some(slot) = g.current() else { return Vec::new() };
    let stops = if slot.is_empty() { "empty".to_string() } else { plural(slot.stops.len(), "stop", "stops") };
    let rows: Vec<Box<dyn Scene>> = vec![
        Box::new(labeled("Name".into(), text_hint(GradientProp::Name, slot.name.clone(), "optional"))),
        Box::new(text(format!("Slot {}, id {}, {stops}", g.slot, slot.id), TEXT_DIM, 12.0)),
    ];
    vec![subpane_box("Gradient", rows)]
}

/// Hotkeys of the gradients editor, shown in its right sidebar.
const GRADIENT_HOTKEYS: [(&str, &str); 8] = [
    ("Click", "Add a stop (above / on the bar)"),
    ("Click stop", "Pick its color (or click its swatch)"),
    ("Drag", "Move a stop / midpoint"),
    ("Arrows", "Nudge stop (Shift: x10)"),
    ("Del", "Delete the selected stop"),
    ("Ctrl+C / V", "Copy / paste the gradient"),
    ("Ctrl+D", "Duplicate the gradient"),
    ("Ctrl+Z / Y", "Undo / redo"),
];

impl TabDoc for GradientsDoc {
    fn apply(&mut self, ctx: &mut DocCtx, action: &TabAction) -> Handled {
        self.apply_edit(ctx, action)
    }

    fn current_value(&self, key: &PropKey, _ctx: &StatusCtx) -> Option<PropValue> {
        self.prop_of(key.downcast()?)
    }

    fn asset(&self) -> Option<&AssetRef> {
        Some(&self.aref)
    }

    fn view(&self) -> ViewKindId {
        GRADIENTS
    }

    fn base_title(&self) -> String {
        asset_title(&self.id)
    }

    fn icon(&self) -> IconRef {
        crate::icons::GRADIENTS
    }

    fn watermark(&self) -> Option<&'static str> {
        Some("gradients")
    }

    fn is_dirty(&self) -> bool {
        GradientsDoc::is_dirty(self)
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn save(&mut self, ctx: &mut DocCtx) -> std::io::Result<()> {
        let project = project_of(ctx)?;
        ops::save_gradients(project, &self.id, &self.collection).map_err(io_error)?;
        self.mark_saved();
        Ok(())
    }

    fn changed_on_disk(&self, project: &Project) -> bool {
        assets::load_gradients(project, &self.id)
            .is_ok_and(|c| GradientCollection::from_ron(&self.saved).ok() != Some(c))
    }

    fn reload_from_disk(&mut self, ctx: &mut DocCtx) -> Result<(), String> {
        let project = ctx.project.ok_or("no project is open")?;
        self.reload(assets::load_gradients(project, &self.id)?);
        Ok(())
    }

    fn keep_mine(&mut self, project: &Project) {
        if let Ok(on_disk) = assets::load_gradients(project, &self.id) {
            self.saved = on_disk.to_ron();
        }
    }

    /// The collection has no name field (its name is the file name): only the id follows.
    fn rename(&mut self, _project: &Project, from: &AssetRef, to: &AssetRef) {
        if self.aref != *from {
            return;
        }
        let Some(to_id) = AssetId::from_ref(to) else { return };
        self.aref = to.clone();
        self.id = to_id;
        self.revision += 1;
    }

    fn config_key(&self) -> Option<String> {
        Some(self.id.source_rel())
    }

    /// `N / 128 used` and the baked texture's size (gradient design §8).
    fn status(&self, ctx: &StatusCtx) -> Option<StatusInfo> {
        let used = self.collection.slots.iter().filter(|s| !s.is_empty()).count();
        let counts = vec![format!("{used} / {SLOTS} used"), format!("{SAMPLES}×{SLOTS} RGBA8")];
        Some(StatusInfo::new(GradientsDoc::is_dirty(self), ctx, counts))
    }
}
