//! Project layer: layout, asset discovery, regeneration pipeline, atomic writes, file ops.
//!
//! Used by both the GUI and the headless CLI; has no Bevy dependency.

pub mod assets;
pub mod backend;
pub mod fixtures;
pub mod game_assets;
pub mod gradient_png;
pub mod layout;
pub mod ops;
pub mod regen;

pub use assets::{AssetId, AssetSet, AssetType};
pub use layout::{MaterialLayout, OpenError, Project, ProjectSettings};
pub use project_core::writer;
pub use regen::{DepGraph, RegenOptions, RegenReport, Scope, regenerate};
