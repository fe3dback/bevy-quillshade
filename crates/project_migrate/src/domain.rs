//! The domain side of the Migration Tool (techdoc §4.4, ADR 0046 "Crate `project_migrate` (D38)"): the
//! [`DomainMigrator`] trait every domain with format steps implements, and the [`MigrationFacts`] store through which
//! a step's `collect` pre-pass publishes what other domains need (Phase 1: `path -> uuid`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use project_core::{DomainId, ProjectVersion};

use crate::{MigrationError, RawProject};

/// The migration steps of one domain's source files. A domain keeps its step code in its Bevy-free backend; a step
/// may use the domain's current DTOs only while it is the latest step (techdoc §4.4 "Rules").
///
/// Per step `target`, the orchestrator calls `collect` for every file of every domain, then `is_fresh` for every
/// file, then `migrate` for the stale ones (D39).
pub trait DomainMigrator: Send + Sync {
    fn domain(&self) -> DomainId;
    /// Source files this domain owns (under `quillshade/<domain>/`), absolute; the orchestrator sorts them.
    fn files(&self, project: &RawProject) -> Vec<PathBuf>;
    /// `false` = the file is stale for project version `target`.
    fn is_fresh(&self, file: &Path, text: &str, target: ProjectVersion) -> Result<bool, MigrationError>;
    /// Optional pre-pass over all of this domain's files: publish facts other domains need at `target`
    /// (e.g. `path -> uuid` for the UUID step). Default: nothing.
    fn collect(&self, _file: &Path, _text: &str, _target: ProjectVersion, _facts: &mut MigrationFacts) {}
    /// Upgrade one stale file by exactly one project version, to `target`.
    fn migrate(
        &self,
        file: &Path,
        text: &str,
        target: ProjectVersion,
        facts: &MigrationFacts,
    ) -> Result<String, MigrationError>;
}

/// A deterministic string store namespaced by the publishing domain, reset for every step (ADR 0046).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationFacts {
    facts: BTreeMap<(DomainId, String), String>,
}

impl MigrationFacts {
    /// Publishes `key = value` under `domain`; returns the previous value of that key.
    pub fn insert(&mut self, domain: DomainId, key: impl Into<String>, value: impl Into<String>) -> Option<String> {
        self.facts.insert((domain, key.into()), value.into())
    }

    /// The value `domain` published under `key`.
    pub fn get(&self, domain: DomainId, key: &str) -> Option<&str> {
        self.facts.get(&(domain, key.to_string())).map(String::as_str)
    }

    /// Every `(key, value)` published by `domain`, ordered by key.
    pub fn iter(&self, domain: DomainId) -> impl Iterator<Item = (&str, &str)> {
        self.facts
            .range((domain, String::new())..)
            .take_while(move |((d, _), _)| *d == domain)
            .map(|((_, k), v)| (k.as_str(), v.as_str()))
    }

    pub fn len(&self) -> usize {
        self.facts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facts_are_namespaced_and_ordered() {
        let (a, b, c) = (DomainId::new("alpha"), DomainId::new("beta"), DomainId::new("gamma"));
        let mut facts = MigrationFacts::default();
        assert!(facts.is_empty());
        facts.insert(b, "z", "1");
        facts.insert(b, "a", "2");
        facts.insert(a, "z", "3");
        facts.insert(c, "", "4");
        assert_eq!(facts.insert(b, "a", "5"), Some("2".to_string()));
        assert_eq!(facts.len(), 4);
        assert_eq!(facts.get(b, "a"), Some("5"));
        assert_eq!(facts.get(a, "a"), None);
        assert_eq!(facts.iter(b).collect::<Vec<_>>(), [("a", "5"), ("z", "1")]);
        assert_eq!(facts.iter(a).collect::<Vec<_>>(), [("z", "3")]);
        assert_eq!(facts.iter(c).collect::<Vec<_>>(), [("", "4")]);
        assert_eq!(facts.iter(DomainId::new("delta")).count(), 0);
    }
}
