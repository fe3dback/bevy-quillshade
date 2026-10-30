//! What every tab's content offers the editor, whatever its kind and domain (decision 0014, design §6.2): identity,
//! title, icon, dirty state, saving, reloading after external changes, following a rename and the key it is
//! remembered by in the user config (decision 0018). [`crate::tab::Tab::doc`] gives it for any tab, so code that
//! only needs one of these operations does not care about the kind; kind-specific work downcasts
//! ([`crate::tab::Tab::get`]). Edits reach the document as [`TabAction`]s ([`TabDoc::apply`]).

use std::any::Any;

use project_core::umbrella::CrateContribution;
use project_core::{AssetRef, Diagnostic, Project, ProjectSettings};

use crate::action::{Handled, TabAction};
use crate::clipboard::Clipboards;
use crate::domain::ToolId;
use crate::dynval::{DynPayload, Payload, PropKey, PropValue};
use crate::message::Message;
use crate::tab::ViewKindId;
use crate::ui::icons::IconRef;

pub trait TabDoc: Any + Send + Sync {
    /// The asset this document edits. `None` for read-only views of something that is not an asset (a generated
    /// file, a game image).
    fn asset(&self) -> Option<&AssetRef>;
    /// The asset the tab belongs to: the one it edits ([`TabDoc::asset`], the default), or the source of a generated
    /// file (closed with it on delete, highlighted in the project tree).
    fn owner(&self) -> Option<&AssetRef> {
        self.asset()
    }
    fn view(&self) -> ViewKindId;
    /// Full title without decorations (`quillshade/material/env/rock_wet.material`,
    /// `assets/quillshade/material/autogen/...wesl`).
    fn base_title(&self) -> String;
    /// Decoration after the title in the secondbar and the window title (` (preview)`); empty by default.
    fn title_suffix(&self) -> &'static str {
        ""
    }
    /// The tab's icon in the secondbar.
    fn icon(&self) -> IconRef;
    /// Watermark in the editor content's corner: the asset type or the language of a generated file; `None` shows
    /// none.
    fn watermark(&self) -> Option<&'static str>;
    fn is_dirty(&self) -> bool;
    fn is_read_only(&self) -> bool;
    /// Bumped whenever the content the view shows changes.
    fn revision(&self) -> u64;
    /// Writes the document and marks it saved; a no-op for read-only documents.
    fn save(&mut self, ctx: &mut DocCtx) -> std::io::Result<()>;
    /// Whether the file differs from what the document last saved or loaded (always `false` for read-only
    /// documents: they are re-read on activation and after regeneration instead).
    fn changed_on_disk(&self, project: &Project) -> bool;
    /// Replaces the content with the file's (history is cleared); `Err` when it does not load.
    fn reload_from_disk(&mut self, ctx: &mut DocCtx) -> Result<(), String>;
    /// "Keep mine" after an external change: the file's version becomes the saved baseline, so the edits stay and
    /// the document stays dirty. Nothing happens when the file does not load; a no-op for read-only documents.
    fn keep_mine(&mut self, project: &Project);
    /// Follows the asset `from` renamed to `to` (already moved on disk); ignored by documents of other assets.
    fn rename(&mut self, project: &Project, from: &AssetRef, to: &AssetRef);
    /// Key in the user config's open tabs, local to the tab's domain (the domain parses it back).
    fn config_key(&self) -> Option<String>;
    /// The tool this document is (design §14.1); `None` (the default) for any other tab. A tool tab is known by the
    /// tool id ([`crate::tab::Tab::key`]) and never written to the user config.
    fn tool(&self) -> Option<ToolId> {
        None
    }
    /// Content of the editor statusbar (design §2.3); `None` (the default) hides it.
    fn status(&self, _ctx: &StatusCtx) -> Option<StatusInfo> {
        None
    }
    /// Carries out an edit (design §6.2). [`Handled::No`] (the default) for a verb the kind does not support, a
    /// property key or a [`TabAction::Doc`] payload of a type it does not know (an edit queued for a tab that is no
    /// longer active): nothing changes. Steps that need more than this document go to [`DocCtx::defer`].
    fn apply(&mut self, _ctx: &mut DocCtx, _action: &TabAction) -> Handled {
        Handled::No
    }
    /// The value a property widget editing `key` shows; `None` (the default) for a key the kind does not know.
    fn current_value(&self, _key: &PropKey, _ctx: &StatusCtx) -> Option<PropValue> {
        None
    }
}

/// What a document's save / reload / edit may use besides itself: the open project, its domain's state and the
/// console/logs; and what an edit asks the editor to do afterwards.
pub struct DocCtx<'a> {
    pub project: Option<&'a Project>,
    /// The state of the tab's domain (`()` when the domain keeps none).
    pub domain: &'a mut dyn Any,
    /// Lines for the console/logs.
    pub messages: &'a mut Vec<Message>,
    /// Set by an edit the right sidebar must show: it is refilled. Merged edits (a drag, typing) leave it alone, as
    /// a refill would take the widget being dragged or typed into.
    pub redraw_props: bool,
    /// Text for the system clipboard (Copy in a read-only text view).
    pub copy_text: Option<String>,
    /// Steps of the edit that need more than the document (other tabs, a confirmation dialog): the editor runs them
    /// after the dispatch, in its domain's code.
    pub deferred: Vec<Payload>,
    /// The editor's in-app clipboards ([`crate::state::editor::Editor::clipboards`], lent for the call; empty in a
    /// context made with [`DocCtx::new`]).
    pub clipboards: Clipboards,
    /// Every domain's module of the umbrella crate ([`crate::domain::DomainList::crate_contributions`]; empty in a
    /// context made with [`DocCtx::new`]): what saving the project settings rewrites it from (design §14.2).
    pub crates: &'a [CrateContribution],
    /// Set by a save that wrote `quillshade/project.ron` (the Project Settings tool): the editor makes them the open
    /// project's settings after the call.
    pub saved_settings: Option<ProjectSettings>,
}

impl<'a> DocCtx<'a> {
    pub fn new(project: Option<&'a Project>, domain: &'a mut dyn Any, messages: &'a mut Vec<Message>) -> Self {
        DocCtx {
            project,
            domain,
            messages,
            redraw_props: false,
            copy_text: None,
            deferred: Vec::new(),
            clipboards: Clipboards::default(),
            crates: &[],
            saved_settings: None,
        }
    }

    /// The domain state as `T`; `None` when it is another type.
    pub fn domain<T: Any>(&mut self) -> Option<&mut T> {
        self.domain.downcast_mut()
    }

    /// Hands a step to the editor ([`DocCtx::deferred`]).
    pub fn defer(&mut self, step: impl DynPayload) {
        self.deferred.push(Payload::new(step));
    }
}

/// What the editor knows about the active tab beyond its document: its asset's diagnostics and its domain's state.
#[derive(Clone, Copy)]
pub struct StatusCtx<'a> {
    /// Diagnostics of the tab's asset only.
    pub diagnostics: &'a [Diagnostic],
    /// The state of the tab's domain (`()` when the domain keeps none).
    pub domain: &'a dyn Any,
}

impl StatusCtx<'_> {
    /// The domain state as `T`; `None` when it is another type.
    pub fn domain<T: Any>(&self) -> Option<&T> {
        self.domain.downcast_ref()
    }
}

impl Default for StatusCtx<'_> {
    fn default() -> Self {
        StatusCtx { diagnostics: &[], domain: &() }
    }
}

/// The editor statusbar's content (design §2.3): only the important bits.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatusInfo {
    /// The document has unsaved changes.
    pub dirty: bool,
    pub errors: usize,
    pub warnings: usize,
    /// Kind-specific counts, shown in order (`3 points`, `340 WESL lines`).
    pub counts: Vec<String>,
}

impl StatusInfo {
    /// Errors / warnings counted from `ctx`.
    pub fn new(dirty: bool, ctx: &StatusCtx, counts: Vec<String>) -> StatusInfo {
        let errors = ctx.diagnostics.iter().filter(|d| d.is_error()).count();
        StatusInfo { dirty, errors, warnings: ctx.diagnostics.len() - errors, counts }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_counts_errors_and_warnings() {
        let diagnostics = [
            Diagnostic::error("E006", "missing"),
            Diagnostic::warning("W101", "unused"),
            Diagnostic::warning("W104", "truncated"),
        ];
        let ctx = StatusCtx { diagnostics: &diagnostics, domain: &() };
        let status = StatusInfo::new(true, &ctx, vec!["3 points".into()]);
        assert_eq!(status, StatusInfo { dirty: true, errors: 1, warnings: 2, counts: vec!["3 points".into()] });
        assert_eq!(StatusInfo::new(false, &StatusCtx::default(), Vec::new()), StatusInfo::default());
    }

    #[test]
    fn contexts_downcast_the_domain_state() {
        let mut state = 7u32;
        let mut messages = Vec::new();
        let mut ctx = DocCtx::new(None, &mut state, &mut messages);
        *ctx.domain::<u32>().expect("u32") += 1;
        assert!(ctx.domain::<String>().is_none());
        ctx.defer(5u8);
        assert_eq!(ctx.deferred, [Payload::new(5u8)]);
        assert_eq!(state, 8);
        let ctx = StatusCtx { diagnostics: &[], domain: &state };
        assert_eq!(ctx.domain::<u32>(), Some(&8));
        assert!(ctx.domain::<i32>().is_none());
    }
}
