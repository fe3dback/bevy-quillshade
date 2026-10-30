//! Colors and text builders shared by the core widgets.

use bevy::prelude::*;

pub const TEXT: Color = Color::srgb(0.86, 0.87, 0.90);
pub const BORDER: Color = Color::srgb_u8(20, 21, 24);

/// Plain text with an explicit color and size.
pub fn text(s: impl Into<String>, color: Color, font_size: f32) -> impl Scene {
    let s = s.into();
    bsn! {
        Text(s)
        TextFont { font_size: FontSize::Px(font_size) }
        TextColor(color)
        Pickable { should_block_lower: false, is_hoverable: false }
    }
}

/// The UI accent (orange): the accent detail of icons, markers of unsaved changes. The only definition of it
/// (`ui::icons` builds its icon palette from [`ACCENT_RGB`]).
pub const ACCENT_RGB: [u8; 3] = [0xfb, 0x92, 0x3c];
pub const ACCENT: Color = Color::srgb_u8(ACCENT_RGB[0], ACCENT_RGB[1], ACCENT_RGB[2]);

/// Secondary text: hints, counts, inactive tabs.
pub const TEXT_DIM: Color = Color::srgb(0.55, 0.57, 0.62);
/// A row under the pointer ([`crate::ui::HoverRow`]).
pub const ROW_HOVER: Color = Color::srgba(1.0, 1.0, 1.0, 0.06);
/// Background of the sidebars and the console/logs.
pub const PANEL_BG: Color = Color::srgb_u8(32, 34, 39);
/// Background of the editor topbar and the editor statusbar (the active tab joins it).
pub const TOPBAR_BG: Color = Color::srgb_u8(30, 32, 37);
/// Background of an inactive asset tab in the secondbar (design §14.3).
pub const TAB_BG: Color = Color::srgb_u8(48, 50, 57);
/// Background of an inactive tool tab (design §14.1, §14.3): a dark blue, apart from the asset tabs and the orange
/// [`ACCENT`].
pub const TOOL_TAB_BG: Color = Color::srgb_u8(34, 48, 78);
/// Background of the active tool tab.
pub const TOOL_TAB_ACTIVE_BG: Color = Color::srgb_u8(46, 68, 112);
/// Text and icon of a greyed tab: one of a real domain other than the active one (design §14.3). [`TEXT_DIM`] made
/// translucent, so it also fades solid icons, whose tint sets only their alpha (`ui::icons::icon`).
pub const TAB_GREY: Color = Color::srgba(0.55, 0.57, 0.62, 0.55);
/// Name of an inactive domain tab in the mainbar (design §14.3): a neutral dim grey on a plain button.
pub const DOMAIN_TAB_TEXT: Color = Color::srgb(0.60, 0.61, 0.64);
/// Name of the active domain tab: near white on Feathers' neutral button fill, brighter than [`TEXT`].
pub const DOMAIN_TAB_ACTIVE_TEXT: Color = Color::srgb(0.96, 0.96, 0.97);
