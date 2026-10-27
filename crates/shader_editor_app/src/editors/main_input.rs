//! Hotkeys through bevy_enhanced_input (design §2.2, decision 0032).
//!
//! One entity, [`InputRoot`], carries every input context: [`MainCtx`] (save, undo / redo, tabs, open project, quit,
//! the View toggles Ctrl+1 / 2 / 3 and `` ` `` Zen mode — always active and evaluated first, priority 2) and one
//! context per editor kind that has hotkeys ([`EditorInput::Ctx`], priority 1), active only while the active tab is of
//! that kind ([`sync_input_context`]).
//! A kind's hotkeys therefore need no "is this tab of my kind" guard.
//!
//! Every action is a [`hotkey`]: it consumes its keys (a `Ctrl+S` action, evaluated before plain ones because it has
//! more modifiers, hides the `S` from them) and requires a reset — a key already down when its context becomes active
//! does nothing until released (bevy_enhanced_input re-arms a binding when it is inserted, so [`sync_input_context`]
//! re-inserts the bindings of a context it activates). Handlers observe `On<Start<A>>` (a [`held_hotkey`]:
//! `On<Fire<A>>`) and push [`state::Action`]s; plain-key handlers ([`on_plain`]) ignore the key while Ctrl, Alt or
//! Super is held, so an unbound combination such as `Ctrl+F` never fires `F`.
//!
//! While the user types — a text field has the focus, the palette, a dialog or a menu is open, a popup (a select's
//! list, the color picker) has the focus — [`gate_keyboard`] hides the keyboard from all contexts; the widgets' own
//! keys (palette list, Enter in a dialog, Space in a select) stay widget-local. The gate runs before the focused
//! input is dispatched, so a key reaches the hotkeys exactly when it does not reach a field. A number input focuses
//! its text on any press, also for a scrub drag, which leaves the text read-only ([`typing_into`]).
//!
//! Keyboard focus is Bevy's default (design §2): a click focuses the widget under it (or clears the focus, on the
//! canvas), Tab / Shift+Tab walk every tabbable widget, buttons included, and Enter / Space press a focused button.
//! Where a key reaches both, the focused widget wins: the gate also holds Space / Enter back from the hotkeys while a
//! button or checkbox has the focus ([`Typing::now`]), so Space presses it instead of opening the palette.
//! [`tab_into_number_input`] only adds to Bevy's Tab, [`drop_hidden_focus`] to its despawn-only focus clearing.

use bevy::camera::visibility::VisibilitySystems;
use bevy::ecs::system::SystemParam;
use bevy::feathers::controls::FeathersNumberInput;
use bevy::input::InputSystems;
use bevy::input_focus::{FocusCause, FocusGained, InputFocus, InputFocusSystems, InputFocusVisible};
use bevy::prelude::*;
use bevy::text::{EditableText, TextEdit, TextReadWriteMode};
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::popover::Popover;
use bevy::ui_widgets::{Button, Checkbox, MenuPopup};
use bevy_enhanced_input::prelude::{
    Action, ActionSettings, ActionSources, Actions, Binding, Bindings, ContextActivity, ContextPriority,
    EnhancedInputPlugin, EnhancedInputSystems, Hold, InputAction, InputContextAppExt, InputModKeys, ModKeys, Start,
};

use super::{EditorAreas, EditorInput, KindVisitor, for_each_kind};
use crate::keys::no_command_mods;
use crate::state::{self, Editor};
use crate::ui::dialog::MenuRoot;
use crate::ui::palette::Palette;

/// Registers the contexts, their handlers and the typing gate. UI side: [`crate::EditorCorePlugin`] runs without it.
/// Adds [`EnhancedInputPlugin`] unless the app has it already.
pub struct InputContextsPlugin;

impl Plugin for InputContextsPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<EnhancedInputPlugin>() {
            app.add_plugins(EnhancedInputPlugin);
        }
        app.init_resource::<KeyboardGate>()
            .add_input_context::<MainCtx>()
            .add_systems(Startup, spawn_input_root)
            // After `InputSystems`: the gate reads this frame's keys and mouse buttons (a Space pressed on a focused
            // button must be held back in the frame it goes down).
            .configure_sets(
                PreUpdate,
                InputSync.after(InputSystems).before(EnhancedInputSystems::Update).before(InputFocusSystems::Dispatch),
            )
            .add_systems(PreUpdate, gate_keyboard.in_set(InputSync))
            .add_observer(tab_into_number_input);
        on_combo::<Save>(app, state::Action::Save);
        on_combo::<SaveAll>(app, state::Action::SaveAll);
        on_combo::<Undo>(app, state::Action::Undo);
        on_combo::<Redo>(app, state::Action::Redo);
        // Resolved against the active tab when applied.
        on_combo::<CloseTab>(app, state::Action::CloseTab(usize::MAX));
        app.init_resource::<DropFocus>()
            .add_observer(next_tab)
            .add_systems(PreUpdate, drop_focus.after(InputFocusSystems::Dispatch).after(EnhancedInputSystems::Trigger))
            .add_systems(PostUpdate, drop_hidden_focus.after(VisibilitySystems::VisibilityPropagate));
        on_combo::<OpenProject>(app, state::Action::OpenProjectDialog);
        on_combo::<Quit>(app, state::Action::Quit);
        on_combo::<ToggleLeftSidebar>(app, state::Action::ToggleLeftSidebar);
        on_combo::<ToggleLog>(app, state::Action::ToggleLog);
        on_combo::<ToggleProperties>(app, state::Action::ToggleProperties);
        on_plain::<ToggleZen>(app, state::Action::ToggleZen);
        on_combo::<EditCopy>(app, state::Action::Copy);
        on_combo::<EditCut>(app, state::Action::Cut);
        on_combo::<EditPaste>(app, state::Action::Paste(None));
        on_combo::<EditDuplicate>(app, state::Action::Duplicate);
        on_combo::<EditSelectAll>(app, state::Action::SelectAll);
        on_plain::<EditDelete>(app, state::Action::DeleteSelection);
        for_each_kind(&mut AddInputs(app));
    }
}

/// Calls every kind's [`EditorAreas::input`].
struct AddInputs<'a>(&'a mut App);

impl KindVisitor for AddInputs<'_> {
    fn visit<E: EditorAreas>(&mut self) {
        E::input(self.0);
    }
}

/// The systems that set up the contexts for this frame's input, before bevy_enhanced_input evaluates them.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct InputSync;

/// The entity carrying every input context.
#[derive(Component, Default, Clone)]
pub struct InputRoot;

/// The global context: works in every editor kind and without a tab.
#[derive(Component, Default, Clone)]
pub struct MainCtx;

#[derive(InputAction)]
#[action_output(bool)]
pub struct Save;

#[derive(InputAction)]
#[action_output(bool)]
pub struct SaveAll;

#[derive(InputAction)]
#[action_output(bool)]
pub struct Undo;

#[derive(InputAction)]
#[action_output(bool)]
pub struct Redo;

#[derive(InputAction)]
#[action_output(bool)]
pub struct CloseTab;

/// Ctrl+Tab: the next tab ([`next_tab`]).
#[derive(InputAction)]
#[action_output(bool)]
pub struct NextTab;

#[derive(InputAction)]
#[action_output(bool)]
pub struct OpenProject;

#[derive(InputAction)]
#[action_output(bool)]
pub struct Quit;

/// Ctrl+1: View → Show project (the left sidebar).
#[derive(InputAction)]
#[action_output(bool)]
pub struct ToggleLeftSidebar;

/// Ctrl+2: View → Show console (the console/logs' messages).
#[derive(InputAction)]
#[action_output(bool)]
pub struct ToggleLog;

/// Ctrl+3: View → Show properties (the right sidebar).
#[derive(InputAction)]
#[action_output(bool)]
pub struct ToggleProperties;

/// `` ` ``: View → Zen mode.
#[derive(InputAction)]
#[action_output(bool)]
pub struct ToggleZen;

// Edit actions several kinds bind in their own contexts; `Editor` routes them to the active tab.

#[derive(InputAction)]
#[action_output(bool)]
pub struct EditCopy;

#[derive(InputAction)]
#[action_output(bool)]
pub struct EditCut;

/// Paste without a position (the graph pastes at the cursor instead, `GraphPaste`).
#[derive(InputAction)]
#[action_output(bool)]
pub struct EditPaste;

#[derive(InputAction)]
#[action_output(bool)]
pub struct EditDuplicate;

#[derive(InputAction)]
#[action_output(bool)]
pub struct EditSelectAll;

/// Delete / Backspace.
#[derive(InputAction)]
#[action_output(bool)]
pub struct EditDelete;

fn main_bindings() -> impl Scene {
    use KeyCode::{Backquote, Digit1, Digit2, Digit3, KeyO, KeyQ, KeyS, KeyW, KeyY, KeyZ, Tab};
    bsn! {
        Actions<MainCtx> [
            @{hotkey::<SaveAll>(ctrl_shift(KeyS))}
            --
            @{hotkey::<Save>(ctrl(KeyS))}
            --
            @{hotkey::<Redo>(ctrl_shift(KeyZ).into_iter().chain(ctrl(KeyY)))}
            --
            @{hotkey::<Undo>(ctrl(KeyZ))}
            --
            @{hotkey::<CloseTab>(ctrl(KeyW))}
            --
            @{hotkey::<NextTab>(ctrl(Tab))}
            --
            @{hotkey::<OpenProject>(ctrl(KeyO))}
            --
            @{hotkey::<Quit>(ctrl(KeyQ))}
            --
            @{hotkey::<ToggleLeftSidebar>(ctrl(Digit1))}
            --
            @{hotkey::<ToggleLog>(ctrl(Digit2))}
            --
            @{hotkey::<ToggleProperties>(ctrl(Digit3))}
            --
            @{hotkey::<ToggleZen>(plain([Backquote]))}
        ]
    }
}

fn spawn_input_root(world: &mut World) {
    let mut root = world.spawn((InputRoot, MainCtx, ContextPriority::<MainCtx>::new(2)));
    if let Err(e) = root.apply_scene(main_bindings()) {
        error!("cannot bind the global hotkeys: {e}");
    }
}

/// Adds an editor kind's context ([`EditorInput`]) to [`InputRoot`] at startup, inactive, and keeps it active exactly
/// while a tab of the kind is ([`sync_input_context`]). Called from the kind's [`EditorAreas::input`].
pub fn add_kind<E: EditorInput + 'static>(app: &mut App) {
    app.add_input_context::<E::Ctx>()
        .add_systems(Startup, attach_kind::<E>.after(spawn_input_root))
        .add_systems(PreUpdate, sync_input_context::<E>.in_set(InputSync).after(gate_keyboard));
}

fn attach_kind<E: EditorInput>(world: &mut World) {
    let Some(root) = world.query_filtered::<Entity, With<InputRoot>>().iter(world).next() else {
        error!("no InputRoot: the hotkeys of the {:?} editor are not bound", E::VIEW);
        return;
    };
    let mut root = world.entity_mut(root);
    root.insert((E::Ctx::default(), ContextPriority::<E::Ctx>::new(1), ContextActivity::<E::Ctx>::INACTIVE));
    if let Err(e) = root.apply_scene(E::bindings()) {
        error!("cannot bind the hotkeys of the {:?} editor: {e}", E::VIEW);
    }
}

/// Activates `E`'s context while the active tab is of kind `E`, deactivates it otherwise. A context it activates, or
/// one active while typing ends, is re-armed: keys already held do nothing until released.
pub fn sync_input_context<E: EditorInput>(
    editor: Res<Editor>,
    gate: Res<KeyboardGate>,
    root: Single<(Entity, &ContextActivity<E::Ctx>), With<InputRoot>>,
    bindings: ContextBindings<E::Ctx>,
    mut commands: Commands,
) {
    let (root, activity) = *root;
    let active = editor.tab().map(|t| t.doc().view()) == Some(E::VIEW);
    if active != **activity {
        let activity = if active { ContextActivity::<E::Ctx>::ACTIVE } else { ContextActivity::INACTIVE };
        commands.entity(root).insert(activity);
    }
    if active && (!**activity || gate.reopened) {
        bindings.rearm(root, &mut commands);
    }
}

/// The bindings of context `C`.
#[derive(SystemParam)]
pub struct ContextBindings<'w, 's, C: Component> {
    actions: Query<'w, 's, &'static Actions<C>>,
    bindings: Query<'w, 's, &'static Bindings>,
    keys: Query<'w, 's, &'static Binding>,
}

impl<C: Component> ContextBindings<'_, '_, C> {
    /// Re-inserts the bindings of the context on `root`: bevy_enhanced_input then ignores each one that is down until
    /// it is released (`require_reset` of a new binding).
    fn rearm(&self, root: Entity, commands: &mut Commands) {
        let Ok(actions) = self.actions.get(root) else { return };
        for action_bindings in self.bindings.iter_many(actions).flatten() {
            for entity in action_bindings {
                if let Ok(binding) = self.keys.get(entity) {
                    commands.entity(entity).insert(*binding);
                }
            }
        }
    }
}

/// Whether the keyboard reaches the contexts ([`gate_keyboard`]).
#[derive(Resource, Default)]
pub struct KeyboardGate {
    pub typing: bool,
    /// Typing ended this frame: active contexts are re-armed.
    pub reopened: bool,
}

/// Whether the user is typing: a text field has the focus ([`typing_into`]), the palette, a dialog or a context menu
/// ([`MenuRoot`]) is open, or the focus is in a popup ([`Popups`]: a main or context menu, a Feathers select's list,
/// the color picker). Also while a focused button or checkbox takes a key it presses with ([`PRESS_KEYS`]).
#[derive(SystemParam)]
pub struct Typing<'w, 's> {
    focus: Option<Res<'w, InputFocus>>,
    fields: Query<'w, 's, Option<&'static TextReadWriteMode>, With<EditableText>>,
    mouse: Option<Res<'w, ButtonInput<MouseButton>>>,
    keys: Option<Res<'w, ButtonInput<KeyCode>>>,
    pressable: Query<'w, 's, (), (Or<(With<Button>, With<Checkbox>)>, Without<InteractionDisabled>)>,
    editor: Res<'w, Editor>,
    palette: Option<Res<'w, Palette>>,
    menus: Query<'w, 's, (), With<MenuRoot>>,
    popups: Popups<'w, 's>,
}

impl Typing<'_, '_> {
    pub fn now(&self) -> bool {
        let held = self.mouse.as_ref().is_some_and(|m| m.pressed(MouseButton::Left));
        let focus = self.focus.as_ref().and_then(|f| f.get());
        self.editor.dialog.is_some()
            || self.palette.as_ref().is_some_and(|p| p.open)
            || !self.menus.is_empty()
            || focus.is_some_and(|f| self.popups.contains(f))
            || focus.and_then(|f| self.fields.get(f).ok()).is_some_and(|m| typing_into(m, held))
            || focus.is_some_and(|f| self.pressable.contains(f))
                && self.keys.as_ref().is_some_and(|k| k.any_pressed(PRESS_KEYS))
    }
}

/// The keys a focused button or checkbox presses with (`bevy_ui_widgets`' `button_on_key_event`,
/// `checkbox_on_key_input`). The widget wins over a hotkey on the same key (design §2): while one is held on it, the
/// gate hides the keyboard from the hotkeys — Space presses the button, the palette stays closed — and re-arms them
/// when it is released. Bevy has no "consumed by the focused widget" for input read outside its dispatch (the widget
/// stops the event's propagation, bevy_enhanced_input reads `ButtonInput`), hence this one-line rule.
pub const PRESS_KEYS: [KeyCode; 2] = [KeyCode::Space, KeyCode::Enter];

/// Popups that hold the keyboard focus while open and close when it leaves them: a menu or select list
/// ([`MenuPopup`]: also our main and context menus), the color picker and other popovers ([`Popover`]).
#[derive(SystemParam)]
pub struct Popups<'w, 's> {
    popups: Query<'w, 's, (), Or<(With<MenuPopup>, With<Popover>)>>,
    parents: Query<'w, 's, &'static ChildOf>,
}

impl Popups<'_, '_> {
    /// `entity` is a popup or inside one.
    pub fn contains(&self, entity: Entity) -> bool {
        self.popups.contains(entity) || self.parents.iter_ancestors(entity).any(|a| self.popups.contains(a))
    }
}

/// A focused text field in `mode` takes the keyboard. A Feathers number input keeps its text focused but read-only
/// (`Static`) while it is scrubbed and after a scrub, Enter or focus loss: during the scrub (the left button `held`)
/// keys stay off the hotkeys, afterwards they reach them again — as they did after a slider drag. Only a click
/// (press + release without movement) makes the text editable, and typing. A number input focused by Tab is made
/// editable by [`tab_into_number_input`] (design §3, decision 0034).
pub fn typing_into(mode: Option<&TextReadWriteMode>, held: bool) -> bool {
    mode != Some(&TextReadWriteMode::Static) || held
}

/// Hides the keyboard from every context while the user is typing. When typing ends it re-arms [`MainCtx`]; the
/// active editor kind's context is re-armed in the same frame by its [`sync_input_context`] (`KeyboardGate::reopened`).
pub fn gate_keyboard(
    typing: Typing,
    mut gate: ResMut<KeyboardGate>,
    mut sources: ResMut<ActionSources>,
    root: Query<Entity, With<InputRoot>>,
    bindings: ContextBindings<MainCtx>,
    mut commands: Commands,
) {
    let typing = typing.now();
    let reopened = gate.typing && !typing;
    if gate.typing != typing || gate.reopened != reopened {
        *gate = KeyboardGate { typing, reopened };
    }
    if sources.keyboard == typing {
        sources.keyboard = !typing;
    }
    if reopened && let Ok(root) = root.single() {
        bindings.rearm(root, &mut commands);
    }
}

/// A number input Tab reaches is made typable (its text all selected), like a fresh one. Feathers leaves its text
/// read-only (`Static`) after a scrub, Enter or focus loss and does not change that on focus, so the keys would
/// neither edit it nor be held back from the hotkeys (decision 0034). It only adds to Bevy's Tab navigation: the focus
/// stays where Bevy put it. Tab is told apart from a click by the focus ring: Bevy 0.20 focuses a clicked `TabIndex`
/// entity with `FocusCause::Navigated` too, but a press hides the ring ([`InputFocusVisible`]) and Tab shows it — a
/// click keeps the scrub behaviour of [`typing_into`].
pub fn tab_into_number_input(
    gained: On<FocusGained>,
    focus: Res<InputFocus>,
    visible: Option<Res<InputFocusVisible>>,
    mut texts: Query<(&mut EditableText, &TextReadWriteMode, &ChildOf)>,
    numbers: Query<(), With<FeathersNumberInput>>,
    mut commands: Commands,
) {
    let e = gained.entity;
    // Only the focused entity's own event, not its ancestors' bubbled copies.
    if e != gained.original_event_target()
        || gained.cause == FocusCause::Auto
        || focus.get() != Some(e)
        || !visible.is_some_and(|v| v.0)
    {
        return;
    }
    if let Ok((mut text, TextReadWriteMode::Static, parent)) = texts.get_mut(e)
        && numbers.contains(parent.parent())
    {
        text.queue_edit(TextEdit::SelectAll);
        commands.entity(e).insert(TextReadWriteMode::Editable);
    }
}

/// Set by [`next_tab`]: [`drop_focus`] clears the keyboard focus this frame.
#[derive(Resource, Default)]
pub struct DropFocus(bool);

/// Ctrl+Tab: the next tab, and the keyboard focus is dropped with its ring ([`drop_focus`]). The one recorded
/// exception to Bevy's default focus (design §2, decision 0035): Bevy's tab navigation ignores Ctrl and would also move
/// the focus to the next widget, where Space / Enter would then press it — but the tab switch replaces the areas the
/// focus was in, so the hotkey wins and nothing keeps the focus.
fn next_tab(_: On<Start<NextTab>>, mut actions: ResMut<state::Actions>, mut drop: ResMut<DropFocus>) {
    actions.push(state::Action::NextTab);
    drop.0 = true;
}

/// Clears the focus [`next_tab`] asked to drop, after Bevy's dispatch (its tab navigation) and the hotkeys ran: they
/// are not ordered against each other.
fn drop_focus(
    mut drop: ResMut<DropFocus>,
    focus: Option<ResMut<InputFocus>>,
    visible: Option<ResMut<InputFocusVisible>>,
) {
    if !std::mem::take(&mut drop.0) {
        return;
    }
    if let Some(mut focus) = focus
        && focus.get().is_some()
    {
        focus.clear();
    }
    if let Some(mut visible) = visible
        && visible.0
    {
        visible.0 = false;
    }
}

/// Clears the keyboard focus when the focused entity goes from visible to hidden: a panel toggle (Ctrl+1 / 3, Zen
/// mode) or a tab switch (an inactive editor area) hid it. An addition to Bevy's default, which clears the focus only
/// when the entity is despawned: a hidden focused button or checkbox would still take Space / Enter (the gate gives
/// them the key, [`PRESS_KEYS`]) and make an edit nobody sees. Only the transition counts, not "hidden now": a popup's
/// item can get the focus while its popover is still hidden for a frame. An entity without `InheritedVisibility`
/// keeps its focus, as Bevy's tab navigation treats it as visible.
pub fn drop_hidden_focus(focus: Option<ResMut<InputFocus>>, visibility: Query<Ref<InheritedVisibility>>) {
    let Some(mut focus) = focus else { return };
    let Some(focused) = focus.get() else { return };
    if let Ok(v) = visibility.get(focused)
        && v.is_changed()
        && !v.is_added()
        && !v.get()
    {
        focus.clear();
    }
}

/// One hotkey action `A` fired by any of `bindings` (see the module docs for the settings).
pub fn hotkey<A: InputAction<Output = bool>>(bindings: impl IntoIterator<Item = Binding>) -> impl Scene {
    let bindings: Vec<_> = bindings.into_iter().map(binding).collect();
    bsn! {
        Action::<A>::new()
        ActionSettings { consume_input: true, require_reset: true }
        Bindings [ {bindings} ]
    }
}

/// A [`hotkey`] that fires once per press after its key has been held `secs` (bevy_enhanced_input's one-shot
/// [`Hold`]), while the key is still down. Observe `On<Fire<A>>`: `Start` comes with the press.
pub fn held_hotkey<A: InputAction<Output = bool>>(
    bindings: impl IntoIterator<Item = Binding>,
    secs: f32,
) -> impl Scene {
    let hold = Hold::new(secs).one_shot(true);
    bsn! { @{hotkey::<A>(bindings)} hold }
}

fn binding(b: Binding) -> impl Scene {
    bsn! { Binding(b) }
}

/// `Ctrl + key`, and `Super + key` the same (macOS Cmd).
pub fn ctrl(key: KeyCode) -> [Binding; 2] {
    [key.with_mod_keys(ModKeys::CONTROL), key.with_mod_keys(ModKeys::SUPER)]
}

/// `Ctrl + Shift + key`, and `Super + Shift + key`.
pub fn ctrl_shift(key: KeyCode) -> [Binding; 2] {
    [key.with_mod_keys(ModKeys::CONTROL | ModKeys::SHIFT), key.with_mod_keys(ModKeys::SUPER | ModKeys::SHIFT)]
}

/// `Shift + key`.
pub fn shift(key: KeyCode) -> Binding {
    key.with_mod_keys(ModKeys::SHIFT)
}

/// Plain keys (Shift allowed, see [`on_plain`]).
pub fn plain<const N: usize>(keys: [KeyCode; N]) -> [Binding; N] {
    keys.map(Binding::from)
}

/// Pushes `action` when `A` (bound to Ctrl / Super combinations) starts.
pub fn on_combo<A: InputAction>(app: &mut App, action: state::Action) {
    app.add_observer(move |_: On<Start<A>>, mut actions: ResMut<state::Actions>| actions.push(action.clone()));
}

/// Pushes `action` when `A` (bound to plain keys) starts, unless Ctrl, Alt or Super is held.
pub fn on_plain<A: InputAction>(app: &mut App, action: state::Action) {
    app.add_observer(move |_: On<Start<A>>, keys: Res<ButtonInput<KeyCode>>, mut actions: ResMut<state::Actions>| {
        if no_command_mods(&keys) {
            actions.push(action.clone());
        }
    });
}
