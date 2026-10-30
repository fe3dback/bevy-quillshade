//! Model-level undo/redo shared by every editable document kind (decision 0008): snapshots of the whole model,
//! consecutive edits with the same key merged into one step.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::dynval::PropKey;

const HISTORY_LIMIT: usize = 200;

/// What makes consecutive edits one undo step: a named gesture (`"drag"`) or the property a widget edits, compared
/// by value ([`PropKey`]'s `PartialEq`), never by identity.
#[derive(Debug, Clone, PartialEq)]
pub enum MergeKey {
    Named(String),
    Prop(PropKey),
}

impl From<&str> for MergeKey {
    fn from(s: &str) -> MergeKey {
        MergeKey::Named(s.to_string())
    }
}

impl From<String> for MergeKey {
    fn from(s: String) -> MergeKey {
        MergeKey::Named(s)
    }
}

impl From<PropKey> for MergeKey {
    fn from(k: PropKey) -> MergeKey {
        MergeKey::Prop(k)
    }
}

#[derive(Debug, Clone)]
pub struct History<T> {
    undo: VecDeque<Arc<T>>,
    redo: Vec<Arc<T>>,
    /// Key of the last merged edit (number scrubbing, typing, a point drag): same key = same undo step.
    merge_key: Option<MergeKey>,
}

impl<T> Default for History<T> {
    fn default() -> Self {
        History { undo: VecDeque::new(), redo: Vec::new(), merge_key: None }
    }
}

impl<T> History<T> {
    /// Ends an edit: `before` is the model it changed (`None` for a no-op edit, which records nothing and leaves
    /// the merge group alone, so a clamped no-op drag cannot join the next real edit to the previous step).
    /// Consecutive edits with the same `key` share one undo step.
    pub fn record(&mut self, before: Option<T>, key: Option<MergeKey>) {
        let Some(before) = before else { return };
        let merged = key.is_some() && self.merge_key == key && !self.undo.is_empty();
        if !merged {
            if self.undo.len() >= HISTORY_LIMIT {
                self.undo.pop_front();
            }
            self.undo.push_back(Arc::new(before));
        }
        self.redo.clear();
        self.merge_key = key;
    }

    /// Ends the current merge group (e.g. a drag finished).
    pub fn end_merge(&mut self) {
        self.merge_key = None;
    }

    /// The state an undo would restore.
    pub fn peek_undo(&self) -> Option<&T> {
        self.undo.back().map(Arc::as_ref)
    }

    /// The state a redo would restore.
    pub fn peek_redo(&self) -> Option<&T> {
        self.redo.last().map(Arc::as_ref)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Swaps `current` with the previous state; `false` when there is none.
    pub fn undo(&mut self, current: &mut T) -> bool
    where
        T: Clone,
    {
        let Some(prev) = self.undo.pop_back() else { return false };
        let prev = Arc::unwrap_or_clone(prev);
        self.redo.push(Arc::new(std::mem::replace(current, prev)));
        self.merge_key = None;
        true
    }

    pub fn redo(&mut self, current: &mut T) -> bool
    where
        T: Clone,
    {
        let Some(next) = self.redo.pop() else { return false };
        let next = Arc::unwrap_or_clone(next);
        self.undo.push_back(Arc::new(std::mem::replace(current, next)));
        self.merge_key = None;
        true
    }

    /// Runs `f` on every stored state, undo and redo alike (a reference the model follows, e.g. a renamed asset, must
    /// not come back with an undo).
    pub fn for_each_snapshot(&mut self, mut f: impl FnMut(&mut T))
    where
        T: Clone,
    {
        for s in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            f(Arc::make_mut(s));
        }
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.merge_key = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Field(&'static str);
    impl crate::dynval::PropTarget for Field {}

    #[test]
    fn equal_keys_merge_into_one_step() {
        let mut h = History::<u32>::default();
        // Two separately built keys with equal contents are one merge group.
        h.record(Some(0), Some(PropKey::new(Field("a")).into()));
        h.record(Some(1), Some(PropKey::new(Field("a")).into()));
        h.record(Some(2), Some("drag".into()));
        h.record(Some(3), Some("drag".into()));
        h.end_merge();
        h.record(Some(4), Some("drag".into()));
        h.record(Some(5), None);
        h.record(Some(6), None);
        let mut cur = 7;
        let mut steps = Vec::new();
        while h.undo(&mut cur) {
            steps.push(cur);
        }
        assert_eq!(steps, [6, 5, 4, 2, 0]);
    }

    #[test]
    fn snapshots_can_be_rewritten() {
        let mut h = History::<u32>::default();
        h.record(Some(1), None);
        h.record(Some(2), None);
        let mut cur = 3;
        assert!(h.undo(&mut cur));
        h.for_each_snapshot(|s| *s *= 10);
        assert!(h.redo(&mut cur));
        assert_eq!(cur, 30, "the redo state");
        assert!(h.undo(&mut cur) && h.undo(&mut cur));
        assert_eq!(cur, 10, "the oldest undo state");
    }
}
