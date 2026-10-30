//! The Light domain (design §9.2): environments (a cube map and its brightness) in `quillshade/light/`, their
//! cube-map Texture assets through `texture_kit`, and the `light` module of the umbrella crate.
//!
//! [`backend`] is Bevy-free (the CLI); with the `editor` feature (default), [`editor`] is the domain the editor
//! registers ([`LightDomain`]). Depends on `editor_core` and `texture_kit` only: never on another domain or a
//! `shader_*` crate.

pub mod backend;

#[cfg(feature = "editor")]
pub mod editor;

#[cfg(feature = "editor")]
pub use editor::{
    ENVIRONMENT_ICON, EnvironmentDoc, EnvironmentEditor, EnvironmentProp, LIGHT_ICON, LIGHT_ICONS, LightDomain,
    LightState, LightTexture,
};
