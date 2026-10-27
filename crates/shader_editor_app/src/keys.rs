//! Modifier keys held now, for every hotkey handler and mouse gesture (design §2.2, decision 0032). Below both
//! [`crate::ui`] and [`crate::editors`]: the input contexts (`editors::main_input`) and the canvas / view gestures
//! (`ui::*`) read modifiers the same way.

use bevy::prelude::*;
use bevy_enhanced_input::prelude::ModKeys;

/// The modifiers held now. The one place mouse gestures read modifiers from.
pub fn mods(keys: &ButtonInput<KeyCode>) -> ModKeys {
    ModKeys::pressed(keys)
}

/// No modifier other than Shift is held: a plain-key action may fire.
pub fn no_command_mods(keys: &ButtonInput<KeyCode>) -> bool {
    !mods(keys).intersects(ModKeys::CONTROL | ModKeys::ALT | ModKeys::SUPER)
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
