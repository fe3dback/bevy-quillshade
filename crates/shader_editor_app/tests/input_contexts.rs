//! Hotkeys through bevy_enhanced_input contexts (design §2.2, decision 0032), without a window: keys go in as
//! `KeyboardInput` messages — what winit sends — so Bevy's `keyboard_input_system` fills `ButtonInput<KeyCode>`, which
//! the input contexts read; the actions they push are recorded before the editor applies them.

use std::collections::BTreeSet;

use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy_enhanced_input::prelude::EnhancedInputPlugin;
use shader_editor_app::editors::main_input::InputContextsPlugin;
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
