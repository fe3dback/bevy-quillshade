//! The built-in project domain (design §14.2): reserved id [`DomainId::PROJECT`], always last in the
//! [`super::DomainList`]. It has no backend, no state, no asset kinds and no `quillshade/project/` folder; its tools
//! are reachable from every domain, and activating one of their tabs leaves the active domain as it is.

use crate::ui::icons::IconId;
use project_core::DomainId;
use project_core::regen::DomainBackend;

use super::registrar::{DomainInfo, DomainRegistrar, EditorDomain};

/// The project domain: `editor_core` appends it to the domain list itself; `src/main.rs` lists only real domains.
pub struct ProjectDomain;

impl EditorDomain for ProjectDomain {
    fn id(&self) -> DomainId {
        DomainId::PROJECT
    }

    fn info(&self) -> DomainInfo {
        DomainInfo {
            title: "Project",
            icon: IconId::Settings.into(),
            summary: "Project — the settings of the whole project, in quillshade/project.ron.",
        }
    }

    fn backend(&self) -> Option<Box<dyn DomainBackend>> {
        None
    }

    /// Its tools: Project Settings (design §14.2).
    fn register(&self, r: &mut DomainRegistrar) {
        crate::settings::register(r);
    }
}
