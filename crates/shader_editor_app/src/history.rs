//! Model-level undo/redo shared by every editable document kind (decision 0008): snapshots of the whole model,
//! consecutive edits with the same key merged into one step.

const HISTORY_LIMIT: usize = 200;

#[derive(Debug, Clone)]
pub struct History<T> {
    undo: Vec<T>,
    redo: Vec<T>,
    /// Key of the last merged edit (number scrubbing, typing, a point drag): same key = same undo step.
    merge_key: Option<String>,
}

impl<T> Default for History<T> {
    fn default() -> Self {
        History { undo: Vec::new(), redo: Vec::new(), merge_key: None }
    }
}

impl<T> History<T> {
    /// Ends an edit: `before` is the model it changed (`None` for a no-op edit, which records nothing and leaves
    /// the merge group alone, so a clamped no-op drag cannot join the next real edit to the previous step).
    /// Consecutive edits with the same `key` share one undo step.
    pub fn record(&mut self, before: Option<T>, key: Option<&str>) {
        let Some(before) = before else { return };
        let merged = key.is_some() && self.merge_key.as_deref() == key && !self.undo.is_empty();
        if !merged {
            self.undo.push(before);
            if self.undo.len() > HISTORY_LIMIT {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.merge_key = key.map(str::to_string);
    }

    /// Ends the current merge group (e.g. a drag finished).
    pub fn end_merge(&mut self) {
        self.merge_key = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Swaps `current` with the previous state; `false` when there is none.
    pub fn undo(&mut self, current: &mut T) -> bool {
        let Some(prev) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(current, prev));
        self.merge_key = None;
        true
    }

    pub fn redo(&mut self, current: &mut T) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(current, next));
        self.merge_key = None;
        true
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.merge_key = None;
    }
}
