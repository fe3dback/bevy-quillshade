//! Domain-neutral project backend (design §4, §6.5): the Quillshade project layout and `project.ron` settings, domain
//! ids, asset UUIDs and the project-wide `AssetIndex` (ADR 0047), diagnostics, autogen markers, atomic writes, RON
//! helpers, the generated umbrella crate, the `DomainBackend` regeneration driver and the project watcher.
//!
//! Knows no asset domain (each one plugs in through `regen::DomainBackend`); depends on no workspace crate and has no
//! Bevy dependency.

pub mod asset_index;
pub mod asset_ref;
pub mod asset_uuid;
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
pub mod version;
pub mod watch;
pub mod writer;

pub use asset_index::{AssetIndex, DomainIndex};
pub use asset_ref::AssetRef;
pub use asset_uuid::{AssetUuid, is_uuid_shaped};
pub use diag::{Diagnostic, Level};
pub use domain::{DomainId, is_valid_id};
pub use layout::{BevyDep, OpenError, Project, ProjectSettings};
pub use ron_io::{LoadError, check_version, probe_version};
pub use version::{
    ProbedVersion, ProjectVersion, VersionProbeError, missing_settings_version, probe_project_version,
    probe_version_key,
};

/// Version of the generator written into autogen headers.
pub const GENERATOR_VERSION: u32 = 1;
