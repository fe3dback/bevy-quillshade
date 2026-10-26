//! Project layer: layout, asset discovery, regeneration pipeline, atomic writes, file ops.
//!
//! Used by both the GUI and the headless CLI; has no Bevy dependency.

pub mod assets;
pub mod fixtures;
pub mod game_assets;
pub mod layout;
pub mod ops;
pub mod regen;
pub mod watch;
pub mod writer;

pub use assets::{AssetId, AssetSet, AssetType};
pub use layout::{OpenError, Project, ProjectSettings};
pub use regen::{DepGraph, RegenOptions, RegenReport, Scope, regenerate};
