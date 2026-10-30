//! Domain-neutral project backend (design §4, §6.5): the Quillshade project layout and `project.ron` settings, domain
//! ids, diagnostics, autogen markers, atomic writes, RON helpers, the generated umbrella crate, the `DomainBackend`
//! regeneration driver and the project watcher.
//!
//! Knows no asset domain (each one plugs in through `regen::DomainBackend`); depends on no workspace crate and has no
//! Bevy dependency.

pub mod asset_ref;
pub mod diag;
pub mod domain;
pub mod files;
pub mod game_files;
pub mod ident;
pub mod layout;
pub mod marker;
pub mod regen;
pub mod ron_io;
pub mod umbrella;
pub mod watch;
pub mod writer;

pub use asset_ref::AssetRef;
pub use diag::{Diagnostic, Level};
pub use domain::{DomainId, is_valid_id};
pub use layout::{BevyDep, OpenError, Project, ProjectSettings};
pub use ron_io::{LoadError, probe_version};

/// Version of the generator written into autogen headers.
pub const GENERATOR_VERSION: u32 = 1;
