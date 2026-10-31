//! Menu data (design §6.1, §8.1): the mainbar's main menus and the items of any menu. Drawing them is the editor's.

use bevy::prelude::Component;

use crate::action::Action;
use crate::ui::icons::IconRef;

/// A main menu of the mainbar. Core fills each one; domains add items through their registration (design §6.1).
#[derive(Component, Default, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MainMenu {
    #[default]
    File,
    Edit,
    View,
    Assets,
    Tools,
    Help,
}

impl MainMenu {
    /// The menus in the mainbar, left to right.
    pub const ORDER: [MainMenu; 6] =
        [MainMenu::File, MainMenu::Edit, MainMenu::View, MainMenu::Assets, MainMenu::Tools, MainMenu::Help];

    pub fn label(self) -> &'static str {
        match self {
            MainMenu::File => "File",
            MainMenu::Edit => "Edit",
            MainMenu::View => "View",
            MainMenu::Assets => "Assets",
            MainMenu::Tools => "Tools",
            MainMenu::Help => "Help",
        }
    }
}

/// An item of a menu (a main menu or a context menu).
#[derive(Clone, Debug, PartialEq)]
pub struct MenuEntry {
    pub icon: Option<IconRef>,
    /// `"Save\tCtrl+S"` shows its hotkey on the right; empty = a separator.
    pub label: String,
    pub action: Action,
    /// Shown dimmed and does nothing; the text (why) is its tooltip.
    pub disabled: Option<String>,
}

impl From<(Option<IconRef>, String, Action)> for MenuEntry {
    fn from((icon, label, action): (Option<IconRef>, String, Action)) -> Self {
        MenuEntry { icon, label, action, disabled: None }
    }
}

/// A menu item that draws a separator line.
pub fn menu_separator() -> (Option<IconRef>, String, Action) {
    (None, String::new(), Action::None)
}
