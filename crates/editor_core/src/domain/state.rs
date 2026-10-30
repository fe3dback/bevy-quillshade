//! The state a domain keeps inside `Editor` (design §6.3) and the hooks core calls on it.

use std::any::Any;
use std::path::PathBuf;

use crate::dynval::Payload;
use project_core::{AssetRef, Diagnostic, DomainId};

use super::TreeAsset;
use crate::state::editor::Editor;

/// State of one domain, stored in [`DomainStates`] (which `Editor` owns), with the hooks core calls in its save /
/// close / rescan / dispatch paths (spec §3.2.4, design §6.3). Each hook that takes `&mut Editor` runs through
/// [`Editor::with_domain`] with the state kept inside `editor.domains`, so `editor.domains.get::<Self>()`,
/// `editor.domains.get_mut::<Self>()` and nested `with_domain` calls find it in place. Every hook does nothing by
/// default.
pub trait DomainState: DomainHooksFor + Any + Send + Sync {
    fn id(&self) -> DomainId;
    /// An [`crate::action::Action::Domain`] payload of this domain, or a step a document of the domain handed
    /// over ([`crate::doc_kind::DocCtx::defer`]); a payload the domain does not know is ignored.
    fn apply(_editor: &mut Editor, _action: &Payload)
    where
        Self: Sized,
    {
    }
    /// After every action that reached a tab of the domain (an edit of its document, closing it, reloading it) or
    /// the domain itself.
    fn after_apply(_editor: &mut Editor)
    where
        Self: Sized,
    {
    }
    /// One save operation wrote these tabs of the domain (empty for a Save All that found none of them dirty): called
    /// once per domain and operation.
    fn after_save(_editor: &mut Editor, _tabs: &[usize])
    where
        Self: Sized,
    {
    }
    /// Tab `tab` of the domain is about to close without saving (closed, the project switched / closed, quit); it is
    /// still in `editor.tabs`.
    fn before_close(_editor: &mut Editor, _tab: usize)
    where
        Self: Sized,
    {
    }
    /// The project's files may have changed (opened, created, renamed, deleted, changed outside, game files added or
    /// removed): re-read them. Also runs when a project opens (state of another project is dropped then) and once it
    /// closed (`editor.project` is `None`: back to the empty state).
    fn rescan(_editor: &mut Editor)
    where
        Self: Sized,
    {
    }
    /// Every diagnostic of the domain (console/logs, editor statusbar, the mainbar's error count), in console/logs
    /// order.
    fn diagnostics(&self) -> Vec<Diagnostic> {
        Vec::new()
    }
    /// [`DomainState::diagnostics`] in the order the console/logs lists them while `active` is the active tab's
    /// asset: by default its diagnostics first.
    fn console_diagnostics(&self, active: Option<&AssetRef>) -> Vec<Diagnostic> {
        let mut all = self.diagnostics();
        if let Some(active) = active {
            let rel = active.source_rel();
            all.sort_by_key(|d| d.asset.as_deref() != Some(rel.as_str()));
        }
        all
    }
    /// The asset rows of the domain's Project tree (design §8.1), in the order they are listed inside each folder
    /// (Material: by kind, then path). `editor` gives open documents (Material: a graph's kind as edited). `None`
    /// (the default): core's rows, the files of the domain's registered asset kinds (design §14.4,
    /// [`crate::state::kinds::default_tree_items`]).
    fn tree_items(&self, _editor: &Editor) -> Option<Vec<TreeAsset>> {
        None
    }
    /// Generate (Assets menu, design §6.5): the domain regenerates everything it generates from what is saved.
    fn generate(_editor: &mut Editor)
    where
        Self: Sized,
    {
    }
    /// The question of the Delete dialog for the domain's asset `r` (what goes with it); `None`: "Delete <file>?".
    fn delete_message(&self, _editor: &Editor, _r: &AssetRef) -> Option<String> {
        None
    }
    /// Renames / moves the file of the domain's asset `r` to the asset path `new_path` (rewriting what refers to it on
    /// disk); the renamed asset, or why it failed. Core then rescans, lets the open tabs follow, logs the rename and
    /// calls [`DomainState::renamed`]. By default the file is moved, nothing else (design §14.4,
    /// [`crate::state::kinds::rename_file`]).
    fn rename_file(editor: &mut Editor, r: &AssetRef, new_path: &str) -> Result<AssetRef, String>
    where
        Self: Sized,
    {
        crate::state::kinds::rename_file(editor, r, new_path)
    }
    /// The domain's asset `from` is now `to` ([`DomainState::rename_file`]): what of the domain refers to it follows
    /// (open documents; core already renamed the navigation history's keys), its output regenerates.
    fn renamed(_editor: &mut Editor, _from: &AssetRef, _to: &AssetRef)
    where
        Self: Sized,
    {
    }
    /// The domain's asset `r` is about to be deleted; its tabs are still open (they close next).
    fn before_delete(_editor: &mut Editor, _r: &AssetRef)
    where
        Self: Sized,
    {
    }
    /// Deletes the file of the domain's asset `r` (rewriting what refers to it on disk); why it failed. By default the
    /// file is removed, nothing else (design §14.4, [`crate::state::kinds::delete_file`]).
    fn delete_file(editor: &mut Editor, r: &AssetRef) -> Result<(), String>
    where
        Self: Sized,
    {
        crate::state::kinds::delete_file(editor, r)
    }
    /// The domain's asset `r` was deleted and the domains rescanned: what referred to it follows, the output
    /// regenerates.
    fn deleted(_editor: &mut Editor, _r: &AssetRef)
    where
        Self: Sized,
    {
    }
    /// A click on one of the domain's diagnostics (a console/logs row, the editor statusbar's count): open its asset
    /// and show where it is.
    fn focus_diagnostic(_editor: &mut Editor, _diag: &Diagnostic)
    where
        Self: Sized,
    {
    }
    /// Files of the project changed outside the editor (`changed`: absolute paths under `quillshade/`, as the watcher
    /// reports them), after the rescan and before the open tabs are compared with their files.
    fn files_changed(_editor: &mut Editor, _changed: &[PathBuf])
    where
        Self: Sized,
    {
    }
}

/// Dispatch table of a [`DomainState`]'s mutable hooks, passed to [`Editor::with_domain`] so the state itself stays
/// inside `editor.domains` while the hook runs.
#[derive(Clone, Copy)]
pub struct DomainHooks {
    apply: fn(&mut Editor, &Payload),
    after_apply: fn(&mut Editor),
    after_save: fn(&mut Editor, &[usize]),
    before_close: fn(&mut Editor, usize),
    rescan: fn(&mut Editor),
    generate: fn(&mut Editor),
    rename_file: fn(&mut Editor, &AssetRef, &str) -> Result<AssetRef, String>,
    renamed: fn(&mut Editor, &AssetRef, &AssetRef),
    before_delete: fn(&mut Editor, &AssetRef),
    delete_file: fn(&mut Editor, &AssetRef) -> Result<(), String>,
    deleted: fn(&mut Editor, &AssetRef),
    focus_diagnostic: fn(&mut Editor, &Diagnostic),
    files_changed: fn(&mut Editor, &[PathBuf]),
}

impl DomainHooks {
    pub fn apply(self, editor: &mut Editor, action: &Payload) {
        (self.apply)(editor, action);
    }

    pub fn after_apply(self, editor: &mut Editor) {
        (self.after_apply)(editor);
    }

    pub fn after_save(self, editor: &mut Editor, tabs: &[usize]) {
        (self.after_save)(editor, tabs);
    }

    pub fn before_close(self, editor: &mut Editor, tab: usize) {
        (self.before_close)(editor, tab);
    }

    pub fn rescan(self, editor: &mut Editor) {
        (self.rescan)(editor);
    }

    pub fn generate(self, editor: &mut Editor) {
        (self.generate)(editor);
    }

    pub fn rename_file(self, editor: &mut Editor, r: &AssetRef, new_path: &str) -> Result<AssetRef, String> {
        (self.rename_file)(editor, r, new_path)
    }

    pub fn renamed(self, editor: &mut Editor, from: &AssetRef, to: &AssetRef) {
        (self.renamed)(editor, from, to);
    }

    pub fn before_delete(self, editor: &mut Editor, r: &AssetRef) {
        (self.before_delete)(editor, r);
    }

    pub fn delete_file(self, editor: &mut Editor, r: &AssetRef) -> Result<(), String> {
        (self.delete_file)(editor, r)
    }

    pub fn deleted(self, editor: &mut Editor, r: &AssetRef) {
        (self.deleted)(editor, r);
    }

    pub fn focus_diagnostic(self, editor: &mut Editor, diag: &Diagnostic) {
        (self.focus_diagnostic)(editor, diag);
    }

    pub fn files_changed(self, editor: &mut Editor, changed: &[PathBuf]) {
        (self.files_changed)(editor, changed);
    }
}

/// Blanket-implemented helper returning the [`DomainHooks`] table of a [`DomainState`].
pub trait DomainHooksFor {
    fn hooks(&self) -> DomainHooks;
}

impl<T: DomainState> DomainHooksFor for T {
    fn hooks(&self) -> DomainHooks {
        DomainHooks {
            apply: T::apply,
            after_apply: T::after_apply,
            after_save: T::after_save,
            before_close: T::before_close,
            rescan: T::rescan,
            generate: T::generate,
            rename_file: T::rename_file,
            renamed: T::renamed,
            before_delete: T::before_delete,
            delete_file: T::delete_file,
            deleted: T::deleted,
            focus_diagnostic: T::focus_diagnostic,
            files_changed: T::files_changed,
        }
    }
}

/// One state per domain id, in insertion order.
#[derive(Default)]
pub struct DomainStates {
    slots: Vec<(DomainId, Box<dyn DomainState>)>,
}

impl DomainStates {
    /// Adds a state; a state with the same id replaces the old one in place.
    pub fn insert(&mut self, state: Box<dyn DomainState>) {
        let id = state.id();
        match self.slots.iter_mut().find(|(i, _)| *i == id) {
            Some(slot) => slot.1 = state,
            None => self.slots.push((id, state)),
        }
    }

    pub fn get<T: DomainState>(&self) -> Option<&T> {
        self.slots.iter().find_map(|(_, s)| (s.as_ref() as &dyn Any).downcast_ref::<T>())
    }

    pub fn get_mut<T: DomainState>(&mut self) -> Option<&mut T> {
        self.slots.iter_mut().find_map(|(_, s)| (s.as_mut() as &mut dyn Any).downcast_mut::<T>())
    }

    pub fn get_dyn(&self, id: DomainId) -> Option<&dyn DomainState> {
        self.slots.iter().find(|(i, _)| *i == id).map(|(_, s)| s.as_ref())
    }

    pub fn get_dyn_mut(&mut self, id: DomainId) -> Option<&mut dyn DomainState> {
        self.slots.iter_mut().find(|(i, _)| *i == id).map(|(_, s)| s.as_mut())
    }

    /// Ids in insertion order.
    pub fn ids(&self) -> impl Iterator<Item = DomainId> + '_ {
        self.slots.iter().map(|(i, _)| *i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct A;
    struct B(u32);
    struct Wrong;

    macro_rules! state {
        ($t:ty, $id:literal) => {
            impl DomainState for $t {
                fn id(&self) -> DomainId {
                    DomainId::new($id)
                }
            }
        };
    }
    state!(A, "a");
    state!(B, "b");
    state!(Wrong, "wrong");

    /// A state counting the calls of a nested hook.
    struct Probe(u32);
    impl DomainState for Probe {
        fn id(&self) -> DomainId {
            DomainId::new("probe")
        }

        fn after_apply(editor: &mut Editor) {
            editor.domains.get_mut::<Probe>().expect("the probe").0 += 1;
        }
    }

    fn probe(e: &Editor) -> Option<u32> {
        e.domains.get::<Probe>().map(|p| p.0)
    }

    /// `with_domain` keeps the state in `editor.domains` throughout the call and across nested `with_domain` calls;
    /// an unknown id runs nothing.
    #[test]
    fn with_domain_keeps_state_in_place() {
        let id = DomainId::new("probe");
        let mut editor = Editor::default();
        editor.domains.insert(Box::new(Probe(1)));
        let ran = editor.with_domain(id, |e, s| {
            assert!(e.domains.get_dyn(id).is_some(), "kept in place during the call");
            assert_eq!(probe(e), Some(1));
            s.after_apply(e);
            assert_eq!(probe(e), Some(2));
            let nested = e.with_domain(id, |e, inner| {
                assert!(e.domains.get_dyn(id).is_some(), "kept in place during nested call");
                inner.after_apply(e);
            });
            assert!(nested.is_some());
            assert_eq!(probe(e), Some(3), "nested call mutated the state in place");
            "ran"
        });
        assert_eq!(ran, Some("ran"));
        assert_eq!(probe(&editor), Some(3));
        let mut called = false;
        assert!(editor.with_domain(DomainId::new("nope"), |_, _| called = true).is_none());
        assert!(!called, "an unknown domain runs nothing");
        assert_eq!(editor.domains.ids().filter(|i| *i == id).count(), 1);
    }

    #[test]
    fn insert_and_lookup() {
        let mut s = DomainStates::default();
        s.insert(Box::new(A));
        s.insert(Box::new(B(1)));
        assert!(s.get::<Wrong>().is_none());
        assert!(s.get::<A>().is_some() && s.get::<B>().is_some());
        assert_eq!(s.ids().collect::<Vec<_>>(), vec![DomainId::new("a"), DomainId::new("b")]);
        s.get_mut::<B>().expect("b").0 = 7;
        assert_eq!(s.get::<B>().expect("b").0, 7);
        s.insert(Box::new(B(9)));
        assert_eq!(s.get::<B>().expect("b").0, 9);
        assert_eq!(s.ids().count(), 2);
        assert!(s.get_dyn(DomainId::new("a")).is_some() && s.get_dyn_mut(DomainId::new("zz")).is_none());
    }
}
