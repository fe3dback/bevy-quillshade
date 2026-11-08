//! The Data domain (ADR 0049): user-defined DTO schemas (`quillshade/data/<path>.dto.ron`), collections of
//! UUIDv7-identified rows (`quillshade/data/<path>.collection.ron`), `.qsd` binary baking, and the `data` module of
//! the generated `quillshade` umbrella crate.
//!
//! [`backend`] is Bevy-free (the CLI and headless pipeline).

pub mod backend;
#[cfg(feature = "editor")]
pub mod editor;

pub use backend::{DATA, DATA_CRATE, DataBackend, DataMigrator, GameAssetKind};
#[cfg(feature = "editor")]
pub use editor::DataDomain;
