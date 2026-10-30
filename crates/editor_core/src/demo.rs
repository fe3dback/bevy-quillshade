//! Debug aid, core half of the scripted mouse / keyboard gestures (`SHADER_EDITOR_INPUT=<name>`, with
//! `SHADER_EDITOR_SCREENSHOT`): the ones about the window shell, `menu_click`, `menu_close`, `tab[:N]` and `hover`
//! (on the secondbar's Back button or the widget whose tooltip starts with `SHADER_EDITOR_INPUT_TIP`). A domain's
//! gestures (Material: its canvas, trees and property widgets) are played by the domain's own system, with [`Emit`]
//! and [`Step`] from here. The input goes through the same window messages winit writes, so picking and the editor
//! see a real gesture; the outcome is logged.
//!
//! `menu_click` clicks the mainbar's menu `SHADER_EDITOR_INPUT_MENU=<name>` (default `file`), `menu_close` then clicks
//! the middle of the window (on the menu's barrier); both log each main menu's open and active (`Pressed`) state
//! (design §1). `tab:N` presses Tab N times (default 1, at most 30) and logs after each press the focused entity, its
//! tooltip and action and whether a `Display::None` ancestor hides it; the screenshot shows the focus ring (design §2).

use bevy::ecs::system::SystemParam;
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
use bevy::input::mouse::{MouseButtonInput, MouseScrollUnit, MouseWheel};
use bevy::input::touch::TouchPhase;
use bevy::prelude::*;
use bevy::ui::ui_transform::UiGlobalTransform;
use bevy::ui_widgets::MenuButton;
use bevy::window::{CursorMoved, PrimaryWindow, WindowEvent};

use crate::menu::MainMenu;
use crate::tooltip::Tooltip;
use crate::ui::OnActivate;

/// First frame of the gesture (the asset opens on frame 2, the screenshot is taken on frame 240).
pub const START: u32 = 160;

/// One step of a scripted gesture, one per frame.
pub enum Step {
    Move(Vec2),
    Press(MouseButton),
    Release(MouseButton),
    /// A key down (`true`) or up.
    Key(KeyCode, bool),
    /// A key typing this character (down and up).
    Type(KeyCode, char),
    /// Wheel notches (positive: up / zoom in).
    Wheel(f32),
    Wait,
}

/// The window messages a gesture is played through.
#[derive(SystemParam)]
pub struct Emit<'w, 's> {
    window: Query<'w, 's, Entity, With<PrimaryWindow>>,
    events: MessageWriter<'w, WindowEvent>,
    moved: MessageWriter<'w, CursorMoved>,
    wheel: MessageWriter<'w, MouseWheel>,
    buttons: MessageWriter<'w, MouseButtonInput>,
    keyboard: MessageWriter<'w, KeyboardInput>,
}

impl Emit<'_, '_> {
    /// Whether the primary window exists (nothing can be played without it).
    pub fn has_window(&self) -> bool {
        self.window.single().is_ok()
    }

    /// Plays `step` as the messages winit would write.
    pub fn play(&mut self, step: Step) {
        let Ok(window) = self.window.single() else { return };
        match step {
            Step::Move(position) => {
                let e = CursorMoved { window, position, delta: None };
                self.moved.write(e.clone());
                self.events.write(e.into());
            }
            Step::Press(button) => {
                let e = MouseButtonInput { button, state: ButtonState::Pressed, window };
                self.buttons.write(e);
                self.events.write(e.into());
            }
            Step::Release(button) => {
                let e = MouseButtonInput { button, state: ButtonState::Released, window };
                self.buttons.write(e);
                self.events.write(e.into());
            }
            Step::Key(key_code, down) => {
                self.keyboard.write(KeyboardInput {
                    key_code,
                    logical_key: Key::Unidentified(NativeKey::Unidentified),
                    state: if down { ButtonState::Pressed } else { ButtonState::Released },
                    text: None,
                    repeat: false,
                    window,
                });
            }
            Step::Type(key_code, c) => {
                for state in [ButtonState::Pressed, ButtonState::Released] {
                    let text = (state == ButtonState::Pressed).then(|| c.to_string().into());
                    let logical_key = Key::Character(c.to_string().into());
                    self.keyboard.write(KeyboardInput { key_code, logical_key, state, text, repeat: false, window });
                }
            }
            Step::Wheel(y) => {
                let e = MouseWheel { unit: MouseScrollUnit::Line, x: 0.0, y, window, phase: TouchPhase::Moved };
                self.wheel.write(e);
                self.events.write(e.into());
            }
            Step::Wait => {}
        }
    }
}

/// Whether core plays the gesture `name` (the domains' systems leave it alone): the shell's gestures, and `hover`
/// unless it is aimed at a pin (`SHADER_EDITOR_INPUT_PIN`, the domain's canvas).
pub fn is_core_gesture(name: &str) -> bool {
    match name {
        "menu_click" | "menu_close" => true,
        "hover" => std::env::var_os("SHADER_EDITOR_INPUT_PIN").is_none(),
        _ => name == "tab" || name.starts_with("tab:"),
    }
}

/// The logical position of a UI node's centre.
pub fn logical(t: &UiGlobalTransform, c: &ComputedNode) -> Vec2 {
    t.translation * c.inverse_scale_factor()
}

/// A left click at `at`.
pub fn click(at: Vec2) -> [Step; 4] {
    [Step::Move(at), Step::Press(MouseButton::Left), Step::Wait, Step::Release(MouseButton::Left)]
}

#[derive(Default)]
pub struct Script {
    frame: u32,
    steps: Vec<Step>,
}

/// Plays the shell's gestures ([`is_core_gesture`]).
#[allow(clippy::too_many_arguments)]
pub fn scripted_input(
    mut s: Local<Script>,
    mut emit: Emit,
    windows: Query<&Window, With<PrimaryWindow>>,
    menus: Query<(&MainMenu, &Children, &UiGlobalTransform, &ComputedNode)>,
    menu_parts: Query<(Option<&Visibility>, Has<bevy::ui::Pressed>, Has<MenuButton>)>,
    tips: Query<(&Tooltip, &UiGlobalTransform, &ComputedNode)>,
    (focus, ring): (Res<bevy::input_focus::InputFocus>, Res<bevy::input_focus::InputFocusVisible>),
    (acts, parents, layout): (Query<&OnActivate>, Query<&ChildOf>, Query<&Node>),
) {
    let Ok(name) = std::env::var("SHADER_EDITOR_INPUT") else { return };
    if !is_core_gesture(&name) || !emit.has_window() {
        return;
    }
    s.frame += 1;
    if s.frame == START {
        s.steps = match name.as_str() {
            "menu_click" | "menu_close" => {
                let wanted = std::env::var("SHADER_EDITOR_INPUT_MENU").unwrap_or_else(|_| "file".into());
                let Some((.., t, c)) = menus.iter().find(|m| m.0.label().eq_ignore_ascii_case(&wanted)) else {
                    error!("demo {name}: no main menu {wanted:?}");
                    return;
                };
                let mut steps: Vec<Step> =
                    click(logical(t, c)).into_iter().chain((0..10).map(|_| Step::Wait)).collect();
                if name == "menu_close" {
                    // Anywhere off the popup lands on the menu's barrier.
                    let middle =
                        windows.single().map_or(Vec2::splat(400.0), |w| Vec2::new(w.width(), w.height()) / 2.0);
                    steps.extend(click(middle));
                }
                steps
            }
            "hover" => {
                let tip = std::env::var("SHADER_EDITOR_INPUT_TIP").unwrap_or_else(|_| "Back (".into());
                let Some((_, t, c)) = tips.iter().find(|t| t.0.0.starts_with(&tip)) else {
                    error!("demo {name}: no widget with the tooltip {tip:?}");
                    return;
                };
                vec![Step::Move(logical(t, c))]
            }
            tab => {
                let n = tab.strip_prefix("tab:").and_then(|n| n.parse().ok()).unwrap_or(1usize).min(30);
                (0..n).flat_map(|_| [Step::Key(KeyCode::Tab, true), Step::Key(KeyCode::Tab, false)]).collect()
            }
        };
        s.steps.reverse();
    }
    if s.frame <= START {
        return;
    }
    match s.steps.pop() {
        Some(step) => {
            if let Step::Key(KeyCode::Tab, false) = step {
                // The focus the press (the frame before) moved.
                let f = focus.get();
                let tip = f.and_then(|f| tips.get(f).ok()).map(|t| t.0.0.clone());
                let action = f.and_then(|f| acts.get(f).ok()).map(|a| a.0.clone());
                let hidden = f.is_some_and(|f| {
                    std::iter::once(f)
                        .chain(parents.iter_ancestors(f))
                        .any(|a| layout.get(a).is_ok_and(|n| n.display == Display::None))
                });
                info!("demo {name}: focus {f:?} tip {tip:?} action {action:?} display-hidden {hidden} ring {}", ring.0);
            }
            emit.play(step);
        }
        None if s.frame == START + 40 && name.starts_with("menu_") => {
            for (menu, children, ..) in &menus {
                // The popup's visibility and the button's `Pressed`.
                let (mut open, mut active) = (false, false);
                for (visibility, pressed, button) in children.iter().filter_map(|c| menu_parts.get(c).ok()) {
                    open |= !button && visibility == Some(&Visibility::Visible);
                    active |= button && pressed;
                }
                info!("demo {name}: menu {} open {open} active {active}", menu.label());
            }
        }
        None => {}
    }
}
