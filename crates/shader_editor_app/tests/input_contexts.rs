//! Hotkeys through bevy_enhanced_input contexts (design §2.2, decision 0032), without a window: keys go in as
//! `KeyboardInput` messages — what winit sends — so Bevy's `keyboard_input_system` fills `ButtonInput<KeyCode>`, which
//! the input contexts read; the actions they push are recorded before the editor applies them.

use std::collections::BTreeSet;

use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy_enhanced_input::prelude::{Actions as InputActions, ContextActivity, EnhancedInputPlugin};
use shader_editor_app::editors::main_input::{InputContextsPlugin, InputRoot};
use shader_editor_app::editors::{CurveEditor, EditorInput, GraphEditor, TextEditor};
use shader_editor_app::state::{Action, Actions, CurveAction, Dialog, Editor, ViewRequest};
use shader_editor_app::tab::{Lang, Tab, ViewKind};
use shader_editor_app::ui::palette::Palette;
use shader_editor_app::{EditorCorePlugin, apply_actions};
use shader_graph_core::{Hotkey, HotkeyKey};
use shader_project::{AssetId, AssetType, Project, fixtures};

/// Actions pushed by the input layer, in order.
#[derive(Resource, Default)]
struct Seen(Vec<Action>);

fn record(actions: Res<Actions>, mut seen: ResMut<Seen>) {
    seen.0.extend(actions.0.iter().cloned());
}

fn material() -> AssetId {
    AssetId::new(AssetType::Material, "env/rock_wet")
}

/// The app of `ui_smoke.rs` minus the canvas entity, plus the input contexts; `setup` opens the tabs.
fn app(setup: impl FnOnce(&mut Editor)) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    let p = Project::open(dir.path()).unwrap();
    fixtures::save_all(&p).unwrap();
    let mut editor = Editor::default();
    editor.open_project(dir.path()).unwrap();
    setup(&mut editor);

    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        bevy::scene::ScenePlugin,
        bevy::input::InputPlugin,
        bevy_noodle::prelude::NoodleCorePlugin,
        EnhancedInputPlugin,
    ));
    app.init_resource::<InputFocus>();
    app.insert_resource(editor);
    app.add_plugins((EditorCorePlugin, InputContextsPlugin));
    app.init_resource::<Seen>().add_systems(Update, record.before(apply_actions));
    app.finish();
    app.update();
    app.update();
    app.world_mut().resource_mut::<Seen>().0.clear();
    (dir, app)
}

fn send(app: &mut App, keys: &[KeyCode], state: ButtonState) {
    for &key_code in keys {
        app.world_mut().write_message(KeyboardInput {
            key_code,
            logical_key: Key::Unidentified(NativeKey::Unidentified),
            state,
            text: None,
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
    }
}

/// Presses `keys` (in this order, in one frame) and runs `frames` updates while they are held.
fn press(app: &mut App, keys: &[KeyCode], frames: usize) {
    send(app, keys, ButtonState::Pressed);
    for _ in 0..frames {
        app.update();
    }
}

fn release(app: &mut App, keys: &[KeyCode]) {
    send(app, keys, ButtonState::Released);
    app.update();
}

/// Taps `keys` together: press, two held frames, release.
fn tap(app: &mut App, keys: &[KeyCode]) {
    press(app, keys, 2);
    release(app, keys);
}

fn seen(app: &mut App) -> Vec<Action> {
    std::mem::take(&mut app.world_mut().resource_mut::<Seen>().0)
}

fn view(app: &App) -> Option<ViewKind> {
    app.world().resource::<Editor>().tab().map(|t| t.doc().view())
}

/// A material tab, then its generated WESL in a text tab; `graph_active` re-activates the material.
fn graph_and_text(graph_active: bool) -> impl FnOnce(&mut Editor) {
    move |e| {
        e.open_asset(&material());
        e.regenerate();
        e.show_generated(&material(), Lang::Wesl);
        assert!(matches!(e.tab(), Some(Tab::Text(_))), "the WESL tab opens");
        if graph_active {
            e.apply(Action::NextTab);
            assert!(e.doc().is_some());
        }
    }
}

#[test]
fn ctrl_s_saves_once_without_plain_s() {
    let (_dir, mut app) = app(|e| e.open_asset(&material()));
    tap(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyS]);
    assert_eq!(seen(&mut app), vec![Action::Save]);
    // Super works as Ctrl (macOS Cmd).
    tap(&mut app, &[KeyCode::SuperLeft, KeyCode::KeyS]);
    assert_eq!(seen(&mut app), vec![Action::Save]);
}

#[test]
fn ctrl_shift_s_saves_all_only() {
    let (_dir, mut app) = app(|e| e.open_asset(&material()));
    tap(&mut app, &[KeyCode::ControlLeft, KeyCode::ShiftLeft, KeyCode::KeyS]);
    assert_eq!(seen(&mut app), vec![Action::SaveAll]);
    tap(&mut app, &[KeyCode::ControlRight, KeyCode::ShiftRight, KeyCode::KeyZ]);
    assert_eq!(seen(&mut app), vec![Action::Redo]);
}

#[test]
fn unbound_combo_does_not_fire_plain_key() {
    let (_dir, mut app) = app(|e| e.open_asset(&material()));
    press(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyF], 2);
    // Letting go of Ctrl first does not turn the held F into a plain F.
    release(&mut app, &[KeyCode::ControlLeft]);
    app.update();
    release(&mut app, &[KeyCode::KeyF]);
    tap(&mut app, &[KeyCode::AltLeft, KeyCode::Delete]);
    assert_eq!(seen(&mut app), vec![]);
    // The plain keys themselves work.
    tap(&mut app, &[KeyCode::KeyF]);
    tap(&mut app, &[KeyCode::Delete]);
    assert_eq!(seen(&mut app), vec![Action::View(ViewRequest::FitSelection), Action::DeleteSelection]);
}

#[test]
fn graph_keys_inactive_on_text_tab() {
    let (_dir, mut app) = app(graph_and_text(false));
    assert_eq!(view(&app), Some(ViewKind::Text));
    tap(&mut app, &[KeyCode::Delete]);
    tap(&mut app, &[KeyCode::KeyF]);
    assert_eq!(seen(&mut app), vec![]);
    tap(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyC]);
    assert_eq!(seen(&mut app), vec![Action::Copy]);
}

#[test]
fn typing_blocks_all_hotkeys() {
    let (_dir, mut app) = app(|e| e.open_asset(&material()));
    let field = app.world_mut().spawn(EditableText::default()).id();
    app.world_mut().insert_resource(InputFocus::from_entity(field));
    app.update();
    tap(&mut app, &[KeyCode::Delete]);
    tap(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyS]);
    tap(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyQ]);
    assert_eq!(seen(&mut app), vec![]);

    // A key held while typing does not fire once the field loses the focus.
    press(&mut app, &[KeyCode::Delete], 1);
    app.world_mut().resource_mut::<InputFocus>().clear();
    app.update();
    app.update();
    release(&mut app, &[KeyCode::Delete]);
    assert_eq!(seen(&mut app), vec![]);
    tap(&mut app, &[KeyCode::Delete]);
    assert_eq!(seen(&mut app), vec![Action::DeleteSelection]);
}

/// A Feathers number input keeps its text focused but read-only (`Static`) during and after a scrub drag: hotkeys
/// stay off only while a mouse button is held.
#[test]
fn read_only_focused_text_blocks_hotkeys_only_while_held() {
    use bevy::input::mouse::MouseButtonInput;
    use bevy::text::TextReadWriteMode;

    let (_dir, mut app) = app(|e| e.open_asset(&material()));
    let field = app.world_mut().spawn((EditableText::default(), TextReadWriteMode::Static)).id();
    app.world_mut().insert_resource(InputFocus::from_entity(field));
    let mouse = |app: &mut App, state| {
        let input = MouseButtonInput { button: MouseButton::Left, state, window: Entity::PLACEHOLDER };
        app.world_mut().write_message(input);
        app.update();
    };
    mouse(&mut app, ButtonState::Pressed);
    tap(&mut app, &[KeyCode::Delete]);
    assert_eq!(seen(&mut app), vec![], "held: a scrub drag");
    mouse(&mut app, ButtonState::Released);
    app.update();
    tap(&mut app, &[KeyCode::Delete]);
    assert_eq!(seen(&mut app), vec![Action::DeleteSelection], "released");
    // Clicked into, the text is editable: typing.
    app.world_mut().entity_mut(field).insert(TextReadWriteMode::Editable);
    app.update();
    tap(&mut app, &[KeyCode::Delete]);
    assert_eq!(seen(&mut app), vec![]);
}

/// An open dialog is typing too (Review Focus 1): no hotkey, global ones included.
#[test]
fn open_dialog_blocks_all_hotkeys() {
    let (_dir, mut app) = app(|e| e.open_asset(&material()));
    let message = Dialog::Message { title: "t".into(), message: "m".into() };
    app.world_mut().resource_mut::<Editor>().dialog = Some(message);
    app.update();
    tap(&mut app, &[KeyCode::Delete]);
    tap(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyS]);
    assert_eq!(seen(&mut app), vec![]);
    // Closed, the same keys act.
    app.world_mut().resource_mut::<Editor>().dialog = None;
    app.update();
    tap(&mut app, &[KeyCode::Delete]);
    tap(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyS]);
    assert_eq!(seen(&mut app), vec![Action::DeleteSelection, Action::Save]);
}

/// The open palette is typing too (its search field; Review Focus 1).
#[test]
fn open_palette_blocks_all_hotkeys() {
    let (_dir, mut app) = app(|e| e.open_asset(&material()));
    let mut palette = Palette::default();
    palette.show(Vec2::ZERO, Vec2::ZERO, None);
    app.world_mut().insert_resource(palette);
    app.update();
    tap(&mut app, &[KeyCode::Delete]);
    tap(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyS]);
    assert_eq!(seen(&mut app), vec![]);
    app.world_mut().resource_mut::<Palette>().open = false;
    app.update();
    tap(&mut app, &[KeyCode::Delete]);
    tap(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyS]);
    assert_eq!(seen(&mut app), vec![Action::DeleteSelection, Action::Save]);
}

#[test]
fn held_key_not_fired_after_tab_switch() {
    let (_dir, mut app) = app(graph_and_text(true));
    assert_eq!(view(&app), Some(ViewKind::Graph));
    tap(&mut app, &[KeyCode::ControlLeft, KeyCode::Tab]);
    assert_eq!(view(&app), Some(ViewKind::Text));

    // Delete is held on the text tab (no meaning there) while the graph tab is clicked: still nothing on the graph.
    press(&mut app, &[KeyCode::Delete], 1);
    app.world_mut().resource_mut::<Actions>().push(Action::NextTab);
    app.update();
    assert_eq!(view(&app), Some(ViewKind::Graph));
    app.update();
    app.update();
    release(&mut app, &[KeyCode::Delete]);
    assert_eq!(seen(&mut app), vec![Action::NextTab, Action::NextTab]);
    // Released and pressed again, it deletes.
    tap(&mut app, &[KeyCode::Delete]);
    assert_eq!(seen(&mut app), vec![Action::DeleteSelection]);

    // The same through Ctrl+Tab, letting go of Ctrl before Delete.
    tap(&mut app, &[KeyCode::ControlLeft, KeyCode::Tab]);
    press(&mut app, &[KeyCode::Delete], 1);
    press(&mut app, &[KeyCode::ControlLeft, KeyCode::Tab], 2);
    release(&mut app, &[KeyCode::ControlLeft, KeyCode::Tab]);
    assert_eq!(view(&app), Some(ViewKind::Graph));
    app.update();
    release(&mut app, &[KeyCode::Delete]);
    assert_eq!(seen(&mut app), vec![Action::NextTab, Action::NextTab]);
}

#[test]
fn ctrl_tab_cycles_while_ctrl_is_held() {
    let (_dir, mut app) = app(graph_and_text(true));
    press(&mut app, &[KeyCode::ControlLeft], 1);
    for view_after in [ViewKind::Text, ViewKind::Graph, ViewKind::Text] {
        tap(&mut app, &[KeyCode::Tab]);
        assert_eq!(view(&app), Some(view_after));
    }
    release(&mut app, &[KeyCode::ControlLeft]);
    assert_eq!(seen(&mut app), vec![Action::NextTab; 3]);
}

#[test]
fn curve_nudge_with_arrows() {
    let id = AssetId::new(AssetType::Curve, "common/pulse");
    let (_dir, mut app) = app(|e| {
        e.open_asset(&id);
        e.apply(Action::Curve(CurveAction::Select(BTreeSet::from([2]))));
    });
    let point = |app: &App| match app.world().resource::<Editor>().tab() {
        Some(Tab::Curve(c)) => c.curve.points[2],
        _ => panic!("curve tab"),
    };
    let (t, v) = point(&app);
    tap(&mut app, &[KeyCode::ArrowUp]);
    let (t1, v1) = point(&app);
    assert_eq!(t1, t);
    assert!((v1 - v - 0.01).abs() < 1e-5, "{v} -> {v1}");
    tap(&mut app, &[KeyCode::ShiftLeft, KeyCode::ArrowUp]);
    let (_, v2) = point(&app);
    assert!((v2 - v1 - 0.1).abs() < 1e-5, "{v1} -> {v2}");
    // Graph keys do nothing here; curve ones do.
    seen(&mut app);
    tap(&mut app, &[KeyCode::KeyF]);
    tap(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyA]);
    assert_eq!(seen(&mut app), vec![Action::SelectAll]);
}

/// The node action hotkeys (decision 0021): Shift + main row or the numpad key alone; `V` toggles the preview on
/// release.
#[test]
fn node_hotkeys_and_preview_key() {
    let (_dir, mut app) = app(|e| {
        e.open_asset(&material());
        let ids: Vec<_> = e.doc().unwrap().asset.nodes.keys().cloned().collect();
        let plus = Hotkey::shift(HotkeyKey::Plus);
        let n = ids
            .into_iter()
            .find(|n| {
                e.apply(Action::Select([n.clone()].into()));
                !e.hotkey_actions(plus).is_empty()
            })
            .expect("a node with a Shift+= conversion");
        e.apply(Action::Select([n].into()));
    });
    let expect = |app: &App, key: Hotkey| app.world().resource::<Editor>().hotkey_actions(key);
    let convert = expect(&app, Hotkey::shift(HotkeyKey::Plus));
    tap(&mut app, &[KeyCode::NumpadAdd]);
    assert_eq!(seen(&mut app), convert);
    tap(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyZ]);
    assert_eq!(seen(&mut app), vec![Action::Undo]);
    assert_eq!(expect(&app, Hotkey::shift(HotkeyKey::Plus)), convert, "undone");
    tap(&mut app, &[KeyCode::Equal]);
    assert_eq!(seen(&mut app), vec![], "= without Shift");
    tap(&mut app, &[KeyCode::ShiftLeft, KeyCode::Equal]);
    assert_eq!(seen(&mut app), convert);
    tap(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyZ]);
    seen(&mut app);
    let preview = expect(&app, Hotkey::plain(HotkeyKey::Letter('v')));
    tap(&mut app, &[KeyCode::KeyV]);
    assert_eq!(seen(&mut app), preview);
    tap(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyV]);
    assert_eq!(seen(&mut app), vec![Action::Paste(None)], "Ctrl+V pastes, no preview toggle");
}

/// The context of kind `E` sits on `InputRoot` with its bindings, active exactly while a tab of the kind is.
fn assert_kind_context<E: EditorInput>(app: &mut App) {
    let w = app.world_mut();
    let root = w.query_filtered::<Entity, With<InputRoot>>().single(w).unwrap();
    let root = w.entity(root);
    assert!(root.contains::<E::Ctx>(), "{:?}: no context on InputRoot (forgotten `add_kind::<Self>`?)", E::VIEW);
    let actions = root.get::<InputActions<E::Ctx>>().map_or(0, |a| a.len());
    assert!(actions > 0, "{:?}: the context has no actions", E::VIEW);
    let active = root.get::<ContextActivity<E::Ctx>>().map(|a| **a);
    let want = app.world().resource::<Editor>().tab().map(|t| t.doc().view()) == Some(E::VIEW);
    assert_eq!(active, Some(want), "{:?}: context activity", E::VIEW);
}

/// Every kind with hotkeys ([`EditorInput`]) attaches its context. A new kind with an `EditorInput` impl goes in this
/// list: its `EditorAreas::input` must call `add_kind::<Self>`.
#[test]
fn every_editor_input_kind_has_its_context() {
    let (_dir, mut app) = app(graph_and_text(true));
    assert_kind_context::<GraphEditor>(&mut app);
    assert_kind_context::<TextEditor>(&mut app);
    assert_kind_context::<CurveEditor>(&mut app);
    // And they follow the active tab.
    tap(&mut app, &[KeyCode::ControlLeft, KeyCode::Tab]);
    assert_eq!(view(&app), Some(ViewKind::Text));
    assert_kind_context::<GraphEditor>(&mut app);
    assert_kind_context::<TextEditor>(&mut app);
    assert_kind_context::<CurveEditor>(&mut app);
}

/// Ctrl+C on a graph copies; the plain C (a comment frame) does not fire with it.
#[test]
fn ctrl_c_on_graph_fires_only_copy() {
    let (_dir, mut app) = app(|e| e.open_asset(&material()));
    tap(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyC]);
    assert_eq!(seen(&mut app), vec![Action::Copy]);
    timed(&mut app);
    hold(&mut app, &[KeyCode::KeyC], 0.6);
    release(&mut app, &[KeyCode::KeyC]);
    // No canvas here, so the cursor is off it: `None`, the visible canvas centre (see `ui_smoke.rs`).
    assert_eq!(seen(&mut app), vec![Action::AddComment(None)], "the held plain C still adds a comment");
}

/// Frames of `FRAME` seconds from now on, so a held key's time is known (bevy_enhanced_input's `Hold` reads it).
fn timed(app: &mut App) {
    use bevy::time::TimeUpdateStrategy;
    app.insert_resource(TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f32(FRAME)));
}

const FRAME: f32 = 0.05;

/// Presses `keys` and keeps them down for `secs` (of [`timed`] frames).
fn hold(app: &mut App, keys: &[KeyCode], secs: f32) {
    press(app, keys, (secs / FRAME).round() as usize);
}

/// Keeps the held keys down `secs` longer.
fn wait(app: &mut App, secs: f32) {
    for _ in 0..(secs / FRAME).round() as usize {
        app.update();
    }
}

fn comment_count(app: &App) -> usize {
    app.world().resource::<Editor>().doc().unwrap().asset.comments.len()
}

/// Design §6.2: a C tap adds no comment; the hotkey needs C held for 0.5 s.
#[test]
fn tap_c_adds_no_comment() {
    let (_dir, mut app) = app(|e| e.open_asset(&material()));
    timed(&mut app);
    let n = comment_count(&app);
    hold(&mut app, &[KeyCode::KeyC], 0.2);
    release(&mut app, &[KeyCode::KeyC]);
    wait(&mut app, 1.0);
    assert_eq!(seen(&mut app), vec![]);
    assert_eq!(comment_count(&app), n);
}

/// Design §6.2: holding C adds one comment once 0.5 s are up, while C is still down, and only one per press.
#[test]
fn hold_c_adds_one_comment() {
    let (_dir, mut app) = app(|e| e.open_asset(&material()));
    timed(&mut app);
    let n = comment_count(&app);
    hold(&mut app, &[KeyCode::KeyC], 0.4);
    assert_eq!(comment_count(&app), n, "not yet");
    wait(&mut app, 0.2);
    assert_eq!(comment_count(&app), n + 1, "at 0.6 s, C still down");
    wait(&mut app, 0.9);
    assert_eq!(comment_count(&app), n + 1, "still one at 1.5 s");
    release(&mut app, &[KeyCode::KeyC]);
    assert_eq!(seen(&mut app), vec![Action::AddComment(None)], "cursor off the canvas: its visible centre");
    // The next press is a new hold.
    hold(&mut app, &[KeyCode::KeyC], 0.6);
    release(&mut app, &[KeyCode::KeyC]);
    assert_eq!(comment_count(&app), n + 2);
}

/// Review Focus 2: Ctrl+C held 0.5 s or longer copies and never adds a comment, Ctrl released first included.
#[test]
fn ctrl_c_held_never_adds_comment() {
    let (_dir, mut app) = app(|e| e.open_asset(&material()));
    timed(&mut app);
    let n = comment_count(&app);
    press(&mut app, &[KeyCode::ControlLeft], 1);
    hold(&mut app, &[KeyCode::KeyC], 0.6);
    release(&mut app, &[KeyCode::ControlLeft]);
    release(&mut app, &[KeyCode::KeyC]);
    assert_eq!(seen(&mut app), vec![Action::Copy]);
    assert_eq!(comment_count(&app), n);
    // C kept down long after Ctrl is let go is still the Ctrl+C press.
    press(&mut app, &[KeyCode::ControlLeft], 1);
    hold(&mut app, &[KeyCode::KeyC], 0.2);
    release(&mut app, &[KeyCode::ControlLeft]);
    wait(&mut app, 1.0);
    release(&mut app, &[KeyCode::KeyC]);
    assert_eq!(seen(&mut app), vec![Action::Copy]);
    assert_eq!(comment_count(&app), n);
}

/// Review Focus 1: C held while a text field has the focus adds nothing, nor once the field lets go of it.
#[test]
fn hold_c_while_typing_adds_nothing() {
    let (_dir, mut app) = app(|e| e.open_asset(&material()));
    timed(&mut app);
    let n = comment_count(&app);
    let field = app.world_mut().spawn(EditableText::default()).id();
    app.world_mut().insert_resource(InputFocus::from_entity(field));
    app.update();
    hold(&mut app, &[KeyCode::KeyC], 0.6);
    assert_eq!(comment_count(&app), n);
    app.world_mut().resource_mut::<InputFocus>().clear();
    wait(&mut app, 1.0);
    release(&mut app, &[KeyCode::KeyC]);
    assert_eq!(seen(&mut app), vec![]);
    assert_eq!(comment_count(&app), n);
}

/// The View flags: (left sidebar, log, properties, Zen mode).
fn flags(app: &App) -> (bool, bool, bool, bool) {
    let c = &app.world().resource::<Editor>().config;
    (c.show_left_sidebar, c.show_log, c.show_properties, c.zen_mode)
}

/// The defaults: every panel shown, Zen off.
const DEFAULT_FLAGS: (bool, bool, bool, bool) = (true, true, true, false);

/// View hotkeys (design §3) are global: `` ` `` toggles Zen mode on a graph tab and with no tab; Ctrl+Tab is Next tab.
#[test]
fn backquote_toggles_zen() {
    let on_graph = |e: &mut Editor| e.open_asset(&material());
    let no_tab = |_: &mut Editor| {};
    for (name, setup) in [("graph tab", &on_graph as &dyn Fn(&mut Editor)), ("no tab", &no_tab)] {
        let (_dir, mut app) = app(setup);
        assert_eq!(flags(&app), DEFAULT_FLAGS, "{name}");
        tap(&mut app, &[KeyCode::Backquote]);
        assert_eq!(seen(&mut app), vec![Action::ToggleZen], "{name}");
        assert_eq!(flags(&app), (true, true, true, true), "{name}");
        tap(&mut app, &[KeyCode::Backquote]);
        assert_eq!(flags(&app), DEFAULT_FLAGS, "{name}");
        tap(&mut app, &[KeyCode::ControlLeft, KeyCode::Tab]);
        assert_eq!(seen(&mut app), vec![Action::ToggleZen, Action::NextTab], "{name}");
        assert_eq!(flags(&app), DEFAULT_FLAGS, "{name}");
        tap(&mut app, &[KeyCode::ControlLeft, KeyCode::Backquote]);
        assert_eq!(seen(&mut app), vec![], "{name}: Ctrl+` is not Zen");
    }
}

/// Ctrl+1 / Ctrl+2 / Ctrl+3 (Super the same, as macOS Cmd) flip the left sidebar, the log and the properties, each
/// once per press (design §3), on a graph tab and with no tab.
#[test]
fn ctrl_digits_toggle_panels() {
    let on_graph = |e: &mut Editor| e.open_asset(&material());
    let no_tab = |_: &mut Editor| {};
    for (name, setup) in [("graph tab", &on_graph as &dyn Fn(&mut Editor)), ("no tab", &no_tab)] {
        let (_dir, mut app) = app(setup);
        tap(&mut app, &[KeyCode::ControlLeft, KeyCode::Digit1]);
        assert_eq!(seen(&mut app), vec![Action::ToggleLeftSidebar], "{name}");
        assert_eq!(flags(&app), (false, true, true, false), "{name}");
        tap(&mut app, &[KeyCode::ControlLeft, KeyCode::Digit2]);
        assert_eq!(seen(&mut app), vec![Action::ToggleLog], "{name}");
        assert_eq!(flags(&app), (false, false, true, false), "{name}");
        tap(&mut app, &[KeyCode::ControlRight, KeyCode::Digit3]);
        assert_eq!(seen(&mut app), vec![Action::ToggleProperties], "{name}");
        assert_eq!(flags(&app), (false, false, false, false), "{name}");
        tap(&mut app, &[KeyCode::SuperLeft, KeyCode::Digit1]);
        assert_eq!(seen(&mut app), vec![Action::ToggleLeftSidebar], "{name}: Super+1");
        assert_eq!(flags(&app), (true, false, false, false), "{name}");
        // Held for many frames: still one flip.
        press(&mut app, &[KeyCode::ControlLeft, KeyCode::Digit2], 10);
        release(&mut app, &[KeyCode::ControlLeft, KeyCode::Digit2]);
        assert_eq!(seen(&mut app), vec![Action::ToggleLog], "{name}: held");
        // In Zen mode the flags still flip.
        tap(&mut app, &[KeyCode::Backquote]);
        tap(&mut app, &[KeyCode::ControlLeft, KeyCode::Digit3]);
        assert_eq!(seen(&mut app), vec![Action::ToggleZen, Action::ToggleProperties], "{name}: Zen");
        assert_eq!(flags(&app), (true, true, true, true), "{name}");
        // Plain digits are no view keys.
        tap(&mut app, &[KeyCode::Digit1]);
        assert_eq!(seen(&mut app), vec![], "{name}: plain 1");
    }
}

/// Tab and Shift+Tab are Bevy's focus navigation only (design §2): no hotkey, the properties stay as they are.
#[test]
fn tab_does_not_toggle_properties() {
    let on_graph = |e: &mut Editor| e.open_asset(&material());
    let no_tab = |_: &mut Editor| {};
    for (name, setup) in [("graph tab", &on_graph as &dyn Fn(&mut Editor)), ("no tab", &no_tab)] {
        let (_dir, mut app) = app(setup);
        tap(&mut app, &[KeyCode::Tab]);
        tap(&mut app, &[KeyCode::ShiftLeft, KeyCode::Tab]);
        assert_eq!(seen(&mut app), vec![], "{name}");
        assert_eq!(flags(&app), DEFAULT_FLAGS, "{name}");
    }
}

/// How often the graph's `OpenPalette` started.
#[derive(Resource, Default)]
struct PaletteKey(usize);

/// The node palette moved from Tab to Space (user decision, design §4); Tab is no hotkey.
#[test]
fn space_opens_palette_tab_does_not() {
    use bevy_enhanced_input::prelude::Start;
    use shader_editor_app::editors::graph::OpenPalette;

    let (_dir, mut app) = app(|e| e.open_asset(&material()));
    app.init_resource::<PaletteKey>().add_observer(|_: On<Start<OpenPalette>>, mut n: ResMut<PaletteKey>| n.0 += 1);
    tap(&mut app, &[KeyCode::Space]);
    assert_eq!(app.world().resource::<PaletteKey>().0, 1, "Space");
    assert_eq!(seen(&mut app), vec![]);
    tap(&mut app, &[KeyCode::Tab]);
    assert_eq!(app.world().resource::<PaletteKey>().0, 1, "not Tab");
    assert_eq!(seen(&mut app), vec![]);
}

/// Review Focus 1: `` ` ``, Ctrl+1 / 2 / 3, Tab and Space change nothing while a text field has the focus, a dialog or
/// the palette is open.
#[test]
fn view_keys_ignored_while_typing() {
    let view_keys = |app: &mut App| {
        tap(app, &[KeyCode::Backquote]);
        for digit in [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3] {
            tap(app, &[KeyCode::ControlLeft, digit]);
            tap(app, &[KeyCode::SuperLeft, digit]);
        }
        tap(app, &[KeyCode::Tab]);
    };
    let (_dir, mut app) = app(|e| e.open_asset(&material()));
    let field = app.world_mut().spawn(EditableText::default()).id();
    app.world_mut().insert_resource(InputFocus::from_entity(field));
    app.update();
    view_keys(&mut app);
    tap(&mut app, &[KeyCode::Space]);
    assert_eq!(seen(&mut app), vec![]);
    assert_eq!(flags(&app), DEFAULT_FLAGS, "text field");
    app.world_mut().resource_mut::<InputFocus>().clear();

    let message = Dialog::Message { title: "t".into(), message: "m".into() };
    app.world_mut().resource_mut::<Editor>().dialog = Some(message);
    app.update();
    view_keys(&mut app);
    assert_eq!(seen(&mut app), vec![]);
    assert_eq!(flags(&app), DEFAULT_FLAGS, "dialog");
    app.world_mut().resource_mut::<Editor>().dialog = None;

    let mut palette = Palette::default();
    palette.show(Vec2::ZERO, Vec2::ZERO, None);
    app.world_mut().insert_resource(palette);
    app.update();
    view_keys(&mut app);
    assert_eq!(seen(&mut app), vec![]);
    assert_eq!(flags(&app), DEFAULT_FLAGS, "palette");
    app.world_mut().resource_mut::<Palette>().open = false;
    app.update();
    tap(&mut app, &[KeyCode::ControlLeft, KeyCode::Digit2]);
    assert_eq!(seen(&mut app), vec![Action::ToggleLog], "typing over: the keys work again");
}

/// Keyboard focus is Bevy's (design §2): whatever Bevy focuses keeps the focus — a button clicked, reached by Tab or
/// focused on purpose (`AutoFocus`), a select's trigger and the items of its open list, a field reached by Tab while
/// nothing was typed, any `TabIndex` entity. Bevy focuses a click with `FocusCause::Navigated` as well, with the focus
/// ring hidden; Tab navigation shows it.
#[test]
fn focus_is_never_dropped() {
    use bevy::input_focus::{FocusCause, InputFocusVisible, process_recorded_focus_changes};
    use bevy::ui_widgets::popover::Popover;
    use bevy::ui_widgets::{Button, MenuItem, MenuPopup};

    use shader_editor_app::editors::main_input::InputSync;

    /// How the focus moves: a click (`click_to_focus` + `acquire_focus_tab_index`), Tab navigation, `AutoFocus`.
    #[derive(Clone, Copy)]
    enum By {
        Click,
        Tab,
        Auto,
    }

    /// A focus change made where Bevy makes it: after the typing gate, as the dispatched key or the pointer does.
    #[derive(Resource, Default)]
    struct Pending(Option<(Entity, By)>);

    let (_dir, mut app) = app(|_| {});
    // What `InputFocusPlugin` adds: focus changes become `FocusGained` / `FocusLost`.
    app.init_resource::<InputFocusVisible>()
        .init_resource::<Pending>()
        .add_systems(PostUpdate, process_recorded_focus_changes)
        .add_systems(
            PreUpdate,
            (|mut p: ResMut<Pending>, mut focus: ResMut<InputFocus>, mut ring: ResMut<InputFocusVisible>| {
                if let Some((e, by)) = p.0.take() {
                    let (cause, shown) = match by {
                        By::Click => (FocusCause::Navigated, false),
                        By::Tab => (FocusCause::Navigated, true),
                        By::Auto => (FocusCause::Auto, ring.0),
                    };
                    ring.0 = shown;
                    focus.set(e, cause);
                }
            })
            .after(InputSync),
        );
    let focused = |app: &mut App, e: Entity, by: By| {
        app.world_mut().resource_mut::<Pending>().0 = Some((e, by));
        app.update();
        app.update();
        app.world().resource::<InputFocus>().get()
    };
    let button = app.world_mut().spawn(Button).id();
    assert_eq!(focused(&mut app, button, By::Click), Some(button), "clicked");
    assert_eq!(focused(&mut app, button, By::Tab), Some(button), "Tab");
    assert!(app.world().resource::<InputFocusVisible>().0, "the focus ring stays on");
    assert_eq!(focused(&mut app, button, By::Auto), Some(button), "AutoFocus");

    // A Feathers select: a `FeathersMenu` with its trigger button next to its list.
    let select = app.world_mut().spawn(bevy::feathers::controls::FeathersMenu).id();
    let trigger = app.world_mut().spawn((Button, ChildOf(select))).id();
    let list = app.world_mut().spawn((MenuPopup::default(), ChildOf(select))).id();
    let item = app.world_mut().spawn((MenuItem, ChildOf(list))).id();
    assert_eq!(focused(&mut app, item, By::Tab), Some(item), "the open list");
    assert_eq!(focused(&mut app, trigger, By::Tab), Some(trigger), "the trigger after the list closed");
    let picker = app.world_mut().spawn(Popover::default()).id();
    let mode = app.world_mut().spawn((Button, ChildOf(picker))).id();
    assert_eq!(focused(&mut app, mode, By::Click), Some(mode), "a button in a popover");
    app.world_mut().resource_mut::<InputFocus>().clear();
    app.update();

    let field = app.world_mut().spawn(EditableText::default()).id();
    let next = app.world_mut().spawn(EditableText::default()).id();
    assert_eq!(focused(&mut app, field, By::Tab), Some(field), "Tab while not typing");
    let other = app.world_mut().spawn_empty().id();
    assert_eq!(focused(&mut app, other, By::Tab), Some(other), "anything, Tab while not typing");
    assert_eq!(focused(&mut app, field, By::Click), Some(field), "clicked into a field");
    assert_eq!(focused(&mut app, next, By::Tab), Some(next), "Tab from a field to the next");
}

/// An open context menu (`MenuRoot`) and the focus in a popup (a menu, a select's list, the color picker) take the
/// keyboard: no View toggle (the open View menu would show stale check marks), no palette.
#[test]
fn open_menus_and_popups_block_hotkeys() {
    use bevy::ui_widgets::popover::Popover;
    use bevy::ui_widgets::{MenuItem, MenuPopup};
    use bevy_enhanced_input::prelude::Start;
    use shader_editor_app::editors::graph::OpenPalette;
    use shader_editor_app::ui::dialog::MenuRoot;

    let (_dir, mut app) = app(|e| e.open_asset(&material()));
    app.init_resource::<PaletteKey>().add_observer(|_: On<Start<OpenPalette>>, mut n: ResMut<PaletteKey>| n.0 += 1);
    let blocked = |app: &mut App, what: &str| {
        app.update();
        tap(app, &[KeyCode::Backquote]);
        tap(app, &[KeyCode::ControlLeft, KeyCode::Digit2]);
        tap(app, &[KeyCode::Tab]);
        tap(app, &[KeyCode::Space]);
        assert_eq!(seen(app), vec![], "{what}");
        assert_eq!(flags(app), DEFAULT_FLAGS, "{what}");
        assert_eq!(app.world().resource::<PaletteKey>().0, 0, "{what}");
    };
    let menu = app.world_mut().spawn(MenuRoot).id();
    blocked(&mut app, "a menu");
    app.world_mut().despawn(menu);

    let list = app.world_mut().spawn(MenuPopup::default()).id();
    let item = app.world_mut().spawn((MenuItem, ChildOf(list))).id();
    app.world_mut().insert_resource(InputFocus::from_entity(item));
    blocked(&mut app, "a select's list");
    let picker = app.world_mut().spawn(Popover::default()).id();
    app.world_mut().insert_resource(InputFocus::from_entity(picker));
    blocked(&mut app, "the color picker");

    app.world_mut().resource_mut::<InputFocus>().clear();
    app.update();
    tap(&mut app, &[KeyCode::Backquote]);
    tap(&mut app, &[KeyCode::Space]);
    assert_eq!(seen(&mut app), vec![Action::ToggleZen], "closed");
    assert_eq!(app.world().resource::<PaletteKey>().0, 1);
}

/// Review Focus 1: typing a name into the rename dialog (design §5.5) fires no hotkey — not Space (palette), `c`
/// (comment) or `` ` `` (log); Tab is no hotkey. Its text input has the focus (`AutoFocus`), as in the editor.
#[test]
fn typing_in_rename_dialog_fires_no_hotkey() {
    use bevy_enhanced_input::prelude::Start;
    use shader_editor_app::editors::graph::OpenPalette;
    use shader_editor_app::state::{PropTarget, Submit};

    let (_dir, mut app) = app(|e| e.open_asset(&material()));
    app.init_resource::<PaletteKey>().add_observer(|_: On<Start<OpenPalette>>, mut n: ResMut<PaletteKey>| n.0 += 1);
    let node = {
        let e = app.world().resource::<Editor>();
        e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "param::Scalar").unwrap().0.clone()
    };
    let target = PropTarget::Param { node, key: "name".into() };
    app.world_mut().resource_mut::<Actions>().push(Action::TextDialog(target.clone()));
    app.update();
    seen(&mut app);
    let dialog = app.world().resource::<Editor>().dialog.clone();
    assert!(matches!(&dialog, Some(Dialog::Input { submit: Submit::SetText(t), .. }) if *t == target), "{dialog:?}");
    let field = app.world_mut().spawn(EditableText::default()).id();
    app.world_mut().insert_resource(InputFocus::from_entity(field));
    app.update();
    let (revision, comments) = {
        let d = app.world().resource::<Editor>().doc().unwrap();
        (d.revision, d.asset.comments.len())
    };
    for key in [KeyCode::Space, KeyCode::Tab, KeyCode::KeyC, KeyCode::Backquote] {
        tap(&mut app, &[key]);
    }
    assert_eq!(seen(&mut app), vec![]);
    assert_eq!(app.world().resource::<PaletteKey>().0, 0, "no palette");
    assert_eq!(flags(&app), DEFAULT_FLAGS, "the panels stay");
    let e = app.world().resource::<Editor>();
    assert_eq!(e.dialog, dialog, "the dialog stays open");
    let d = e.doc().unwrap();
    assert_eq!((d.revision, d.asset.comments.len()), (revision, comments), "no comment, no edit");
}

/// Space held on a focused button presses the button, not the palette (design §2). Clicking the canvas while Space is
/// still held clears the focus and reopens the keyboard gate: every active context is re-armed — the graph's too, not
/// only `MainCtx` — so the held Space does not start `OpenPalette`; released and pressed again, it does.
#[test]
fn held_space_from_button_does_not_open_palette_after_focus_clears() {
    use bevy::ui_widgets::Button;
    use bevy_enhanced_input::prelude::Start;
    use shader_editor_app::editors::graph::OpenPalette;
    use shader_editor_app::editors::main_input::KeyboardGate;

    let (_dir, mut app) = app(|e| e.open_asset(&material()));
    app.init_resource::<PaletteKey>().add_observer(|_: On<Start<OpenPalette>>, mut n: ResMut<PaletteKey>| n.0 += 1);
    let button = app.world_mut().spawn(Button).id();
    app.world_mut().insert_resource(InputFocus::from_entity(button));
    app.update();
    press(&mut app, &[KeyCode::Space], 2);
    assert!(app.world().resource::<KeyboardGate>().typing, "Space held on a focused button closes the gate");
    app.world_mut().resource_mut::<InputFocus>().clear();
    app.update();
    assert!(!app.world().resource::<KeyboardGate>().typing, "the focus cleared reopens the gate");
    app.update();
    app.update();
    release(&mut app, &[KeyCode::Space]);
    assert_eq!(app.world().resource::<PaletteKey>().0, 0, "the held Space opens no palette");
    tap(&mut app, &[KeyCode::Space]);
    assert_eq!(app.world().resource::<PaletteKey>().0, 1, "pressed again, it does");
    assert_eq!(seen(&mut app), vec![]);
}
