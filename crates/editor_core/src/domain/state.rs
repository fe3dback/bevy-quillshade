//! The state a domain keeps inside `Editor` (design §6.3) and the hooks core calls on it.

use std::any::Any;
use std::path::PathBuf;

use crate::dynval::Payload;
use project_core::{AssetRef, Diagnostic, DomainId};

use super::TreeAsset;
use crate::state::editor::Editor;

/// State of one domain, stored in [`DomainStates`] (which `Editor` owns), with the hooks core calls in its save /
/// close / rescan / dispatch paths (spec §3.2.4, design §6.3). Each hook that takes `&mut Editor` runs through
/// [`Editor::with_domain`]: the state is taken out of `editor.domains` for the call and put back afterwards, so the
/// hook gets the editor and its own state mutably. A domain whose `Editor`-side code reads its state through
/// `editor.domains` may lend it back there for the duration of the hook (Material: `MaterialState::lent`); `self` is
/// then a placeholder until the lend ends, and the hook must take the state back out before it returns. Every hook
/// does nothing by default.
pub trait DomainState: Any + Send + Sync {
    fn id(&self) -> DomainId;
    /// An [`crate::action::Action::Domain`] payload of this domain, or a step a document of the domain handed
    /// over ([`crate::doc_kind::DocCtx::defer`]); a payload the domain does not know is ignored.
    fn apply(&mut self, _editor: &mut Editor, _action: &Payload) {}
    /// After every action that reached a tab of the domain (an edit of its document, closing it, reloading it) or
    /// the domain itself.
    fn after_apply(&mut self, _editor: &mut Editor) {}
    /// One save operation wrote these tabs of the domain (empty for a Save All that found none of them dirty): called
    /// once per domain and operation.
    fn after_save(&mut self, _editor: &mut Editor, _tabs: &[usize]) {}
    /// Tab `tab` of the domain is about to close without saving (closed, the project switched / closed, quit); it is
    /// still in `editor.tabs`.
    fn before_close(&mut self, _editor: &mut Editor, _tab: usize) {}
    /// The project's files may have changed (opened, created, renamed, deleted, changed outside, game files added or
    /// removed): re-read them. Also runs when a project opens (state of another project is dropped then) and once it
    /// closed (`editor.project` is `None`: back to the empty state).
    fn rescan(&mut self, _editor: &mut Editor) {}
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
    fn generate(&mut self, _editor: &mut Editor) {}
    /// The question of the Delete dialog for the domain's asset `r` (what goes with it); `None`: "Delete <file>?".
    fn delete_message(&self, _editor: &Editor, _r: &AssetRef) -> Option<String> {
        None
    }
    /// Renames / moves the file of the domain's asset `r` to the asset path `new_path` (rewriting what refers to it on
    /// disk); the renamed asset, or why it failed. Core then rescans, lets the open tabs follow, logs the rename and
    /// calls [`DomainState::renamed`]. By default the file is moved, nothing else (design §14.4,
    /// [`crate::state::kinds::rename_file`]).
    fn rename_file(&mut self, editor: &mut Editor, r: &AssetRef, new_path: &str) -> Result<AssetRef, String> {
        crate::state::kinds::rename_file(editor, r, new_path)
    }
    /// The domain's asset `from` is now `to` ([`DomainState::rename_file`]): what of the domain refers to it follows
    /// (open documents; core already renamed the navigation history's keys), its output regenerates.
    fn renamed(&mut self, _editor: &mut Editor, _from: &AssetRef, _to: &AssetRef) {}
    /// The domain's asset `r` is about to be deleted; its tabs are still open (they close next).
    fn before_delete(&mut self, _editor: &mut Editor, _r: &AssetRef) {}
    /// Deletes the file of the domain's asset `r` (rewriting what refers to it on disk); why it failed. By default the
    /// file is removed, nothing else (design §14.4, [`crate::state::kinds::delete_file`]).
    fn delete_file(&mut self, editor: &mut Editor, r: &AssetRef) -> Result<(), String> {
        crate::state::kinds::delete_file(editor, r)
    }
    /// The domain's asset `r` was deleted and the domains rescanned: what referred to it follows, the output
    /// regenerates.
    fn deleted(&mut self, _editor: &mut Editor, _r: &AssetRef) {}
    /// A click on one of the domain's diagnostics (a console/logs row, the editor statusbar's count): open its asset
    /// and show where it is.
    fn focus_diagnostic(&mut self, _editor: &mut Editor, _diag: &Diagnostic) {}
    /// Files of the project changed outside the editor (`changed`: absolute paths under `quillshade/`, as the watcher
    /// reports them), after the rescan and before the open tabs are compared with their files.
    fn files_changed(&mut self, _editor: &mut Editor, _changed: &[PathBuf]) {}
}

/// One state per domain id, in insertion order. A taken state leaves its slot empty so [`DomainStates::put_back`]
/// restores the order.
#[derive(Default)]
pub struct DomainStates {
    slots: Vec<(DomainId, Option<Box<dyn DomainState>>)>,
}

impl DomainStates {
    /// Adds a state; a state with the same id replaces the old one in place.
    pub fn insert(&mut self, state: Box<dyn DomainState>) {
        let id = state.id();
        match self.slots.iter_mut().find(|(i, _)| *i == id) {
            Some(slot) => slot.1 = Some(state),
            None => self.slots.push((id, Some(state))),
        }
    }

    pub fn get<T: DomainState>(&self) -> Option<&T> {
        self.slots.iter().filter_map(|(_, s)| s.as_ref()).find_map(|s| (s.as_ref() as &dyn Any).downcast_ref::<T>())
    }

    pub fn get_mut<T: DomainState>(&mut self) -> Option<&mut T> {
        self.slots
            .iter_mut()
            .filter_map(|(_, s)| s.as_mut())
            .find_map(|s| (s.as_mut() as &mut dyn Any).downcast_mut::<T>())
    }

    pub fn get_dyn(&self, id: DomainId) -> Option<&dyn DomainState> {
        self.slots.iter().find(|(i, _)| *i == id).and_then(|(_, s)| s.as_deref())
    }

    pub fn get_dyn_mut(&mut self, id: DomainId) -> Option<&mut dyn DomainState> {
        match self.slots.iter_mut().find(|(i, _)| *i == id) {
            Some((_, Some(s))) => Some(s.as_mut()),
            _ => None,
        }
    }

    /// Removes a state (so it can be used next to `&mut Editor`); [`DomainStates::put_back`] returns it.
    pub fn take(&mut self, id: DomainId) -> Option<Box<dyn DomainState>> {
        self.slots.iter_mut().find(|(i, _)| *i == id).and_then(|(_, s)| s.take())
    }

    pub fn put_back(&mut self, state: Box<dyn DomainState>) {
        self.insert(state);
    }

    /// Ids in insertion order (a taken state keeps its place).
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
    state!(Probe, "probe");

    fn probe(e: &Editor) -> Option<u32> {
        e.domains.get::<Probe>().map(|p| p.0)
    }

    /// `with_domain` takes the state out for the call; a domain may lend it back (as Material does) and a nested
    /// `with_domain` then takes the lent one, all unwinding with the real state back in place; an unknown id runs
    /// nothing.
    #[test]
    fn with_domain_takes_and_lends() {
        let id = DomainId::new("probe");
        let mut editor = Editor::default();
        editor.domains.insert(Box::new(Probe(1)));
        let ran = editor.with_domain(id, |e, s| {
            assert!(e.domains.get_dyn(id).is_none(), "taken out for the call");
            let s = (s as &mut dyn Any).downcast_mut::<Probe>().expect("the probe");
            assert_eq!(s.0, 1);
            // Lend it back: `s` is a placeholder meanwhile.
            e.domains.put_back(Box::new(std::mem::replace(s, Probe(0))));
            assert_eq!(probe(e), Some(1));
            let nested = e.with_domain(id, |e, inner| {
                assert!(e.domains.get_dyn(id).is_none(), "the nested call takes the lent state");
                (inner as &mut dyn Any).downcast_mut::<Probe>().expect("the probe").0 += 1;
            });
            assert!(nested.is_some());
            assert_eq!(probe(e), Some(2), "the nested call put it back");
            let back = e.domains.take(id).expect("the lent state");
            *s = *(back as Box<dyn Any>).downcast::<Probe>().unwrap_or_else(|_| panic!("a probe"));
            "ran"
        });
        assert_eq!(ran, Some("ran"));
        assert_eq!(probe(&editor), Some(2), "the real state is back in place");
        let mut called = false;
        assert!(editor.with_domain(DomainId::new("nope"), |_, _| called = true).is_none());
        assert!(!called, "an unknown domain runs nothing");
        assert_eq!(editor.domains.ids().filter(|i| *i == id).count(), 1);
    }

    #[test]
    fn take_and_put_back() {
        let mut s = DomainStates::default();
        s.insert(Box::new(A));
        s.insert(Box::new(B(1)));
        assert!(s.get::<Wrong>().is_none());
        let a = s.take(DomainId::new("a")).expect("a");
        assert!(s.get::<A>().is_none() && s.get::<B>().is_some());
        assert!(s.get_dyn(DomainId::new("a")).is_none());
        s.put_back(a);
        assert_eq!(s.ids().collect::<Vec<_>>(), vec![DomainId::new("a"), DomainId::new("b")]);
        s.get_mut::<B>().expect("b").0 = 7;
        assert_eq!(s.get::<B>().expect("b").0, 7);
        s.insert(Box::new(B(9)));
        assert_eq!(s.ids().count(), 2);
        assert!(s.get_dyn(DomainId::new("a")).is_some() && s.get_dyn_mut(DomainId::new("zz")).is_none());
    }
}
