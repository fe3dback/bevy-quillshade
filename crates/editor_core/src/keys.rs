//! Modifier keys held now, for every hotkey handler and mouse gesture (design §2.2, decision 0032). Below both
//! the UI and the editors: the input contexts (`editors::main_input`) and the canvas / view gestures
//! (`ui::*`) read modifiers the same way.

use bevy::prelude::*;
use bevy::text::TextReadWriteMode;
use bevy_enhanced_input::prelude::ModKeys;

/// The modifiers held now. The one place mouse gestures read modifiers from.
pub fn mods(keys: &ButtonInput<KeyCode>) -> ModKeys {
    ModKeys::pressed(keys)
}

/// No modifier other than Shift is held: a plain-key action may fire.
pub fn no_command_mods(keys: &ButtonInput<KeyCode>) -> bool {
    !mods(keys).intersects(ModKeys::CONTROL | ModKeys::ALT | ModKeys::SUPER)
}

/// A focused text field in `mode` takes the keyboard. A Feathers number input keeps its text focused but read-only
/// (`Static`) while it is scrubbed and after a scrub, Enter or focus loss: during the scrub (the left button `held`)
/// keys stay off the hotkeys, afterwards they reach them again — as they did after a slider drag. Only a click
/// (press + release without movement) makes the text editable, and typing. A number input focused by Tab is made
/// editable by `main_input::tab_into_number_input` (design §3, decision 0034).
pub fn typing_into(mode: Option<&TextReadWriteMode>, held: bool) -> bool {
    mode != Some(&TextReadWriteMode::Static) || held
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mods_and_plain_keys() {
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::ShiftLeft);
        assert_eq!(mods(&keys), ModKeys::SHIFT);
        assert!(no_command_mods(&keys), "Shift alone keeps plain keys");
        keys.press(KeyCode::SuperRight);
        assert!(!no_command_mods(&keys));
    }
}
