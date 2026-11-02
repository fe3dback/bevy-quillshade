//! Errors of the Migration Tool (ADR 0046 "Crate `project_migrate` (D38)"). Messages are single-line.

use std::path::{Path, PathBuf};

use project_core::{DomainId, OpenError, ProjectVersion};

/// Why a migration (or its dry run) could not be planned or performed.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    /// The project cannot be opened: not a directory, no `Cargo.toml` / `assets/`, or a `project.ron` that cannot be
    /// read or has no readable project version ([`OpenError::Settings`]).
    #[error(transparent)]
    Open(#[from] OpenError),
    /// The project was written by a newer build (`supported` = the migration target, normally
    /// [`ProjectVersion::CURRENT`]).
    #[error("project version {found} is newer than this editor supports ({supported}); update the editor")]
    TooNew { found: ProjectVersion, supported: ProjectVersion },
    /// Reading or writing a file failed.
    #[error("cannot access `{}`: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// A domain migrator failed on one of its files.
    #[error("cannot migrate `{}` (domain `{domain}`): {message}", path.display())]
    Domain { domain: DomainId, path: PathBuf, message: String },
}

impl MigrationError {
    /// A [`MigrationError::Domain`]; `message` should be a single line.
    pub fn domain(domain: DomainId, path: &Path, message: impl Into<String>) -> MigrationError {
        MigrationError::Domain { domain, path: path.to_path_buf(), message: message.into() }
    }

    /// A [`MigrationError::Io`].
    pub fn io(path: &Path, source: std::io::Error) -> MigrationError {
        MigrationError::Io { path: path.to_path_buf(), source }
    }
}
