//! Navigation history (Back / Forward): one flat list of visited tabs and graph camera views with a cursor, like a
//! browser. New entries only on a tab change or a large camera jump; small moves amend the current entry. In memory
//! only, decision 0041.
//! Generic over the key a tab is remembered by (the editor uses `crate::tab::TabKey`).

use bevy::math::Vec2;

use crate::tab::TabKey;

/// A graph camera: the graph-space point at the middle of the visible canvas and the zoom. The centre, not the raw
/// pan, so a canvas resize is not a camera change.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavView {
    pub centre: Vec2,
    pub zoom: f32,
}

/// One place in the history: a tab and, for a graph tab once its camera is known, its view.
#[derive(Debug, Clone, PartialEq)]
pub struct NavEntry<K> {
    pub tab: K,
    pub view: Option<NavView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavDir {
    Back,
    Forward,
}

/// What happened when navigation tried an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// The entry's tab is now active.
    Opened,
    /// The entry's target no longer exists: the entry is removed.
    Gone,
    /// The target exists but did not open (e.g. it failed to load): the entry is kept.
    Failed,
}

/// Most entries kept; the oldest go first.
pub const NAV_CAP: usize = 100;
/// A camera jump: the centre moved by more than this share of the canvas's smaller side (in screen px)…
pub const NAV_JUMP_SCREEN: f32 = 0.5;
/// …or the zoom changed by at least this factor.
pub const NAV_JUMP_ZOOM: f32 = 2.0;

/// The Back / Forward list with its cursor, over the tab key `K` the app identifies tabs by.
#[derive(Debug, Clone)]
pub struct NavHistory<K> {
    entries: Vec<NavEntry<K>>,
    /// The current entry (meaningless while `entries` is empty).
    cursor: usize,
    /// The entry the cursor last moved away from (`visit` / `step`), for [`NavHistory::amend_last_of`].
    left: Option<usize>,
}

impl<K> Default for NavHistory<K> {
    fn default() -> Self {
        NavHistory { entries: Vec::new(), cursor: 0, left: None }
    }
}

impl<K: Clone + PartialEq> NavHistory<K> {
    pub fn current(&self) -> Option<&NavEntry<K>> {
        self.entries.get(self.cursor)
    }

    pub fn entries(&self) -> &[NavEntry<K>] {
        &self.entries
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The active tab became `tab`: no-op if the current entry is that tab; else truncate forward, push
    /// `{tab, None}`, cap.
    pub fn visit(&mut self, tab: K) {
        if self.current().is_some_and(|e| e.tab == tab) {
            return;
        }
        self.left = (!self.entries.is_empty()).then_some(self.cursor);
        self.push(NavEntry { tab, view: None });
    }

    /// A settled camera of `tab` (`min_side`: the canvas's smaller side, logical px). Ignored unless the current
    /// entry is `tab`. Sets a missing view, pushes `{tab, Some(view)}` on a jump (truncating forward), else amends.
    pub fn note_view(&mut self, tab: &K, view: NavView, min_side: f32) {
        let Some(entry) = self.entries.get_mut(self.cursor).filter(|e| e.tab == *tab) else {
            return;
        };
        match entry.view {
            Some(old) if is_jump(old, view, min_side) => {
                let tab = entry.tab.clone();
                self.push(NavEntry { tab, view: Some(view) });
            }
            _ => entry.view = Some(view),
        }
    }

    /// The last view of `tab` before leaving it: sets the view of the entry the cursor last moved away from
    /// (tracked by `visit` / `step`, kept right across removals and the cap) if its tab is `tab`; else nothing.
    /// No push.
    pub fn amend_last_of(&mut self, tab: &K, view: NavView) {
        if let Some(entry) = self.left.and_then(|i| self.entries.get_mut(i)).filter(|e| e.tab == *tab) {
            entry.view = Some(view);
        }
    }

    /// Whether Back has an entry to try (validity is not checked). With `active` not the current entry's tab
    /// (e.g. no tab open) that is the current entry itself.
    pub fn can_back(&self, active: Option<&K>) -> bool {
        !self.entries.is_empty() && (self.cursor > 0 || !self.on_current(active))
    }

    /// Whether Forward has an entry to try (validity is not checked).
    pub fn can_forward(&self) -> bool {
        self.cursor + 1 < self.entries.len()
    }

    /// Walks from the cursor in `dir`. `try_open` is called per candidate. `Gone` removes the entry and the walk
    /// goes on, `Failed` keeps it and goes on, `Opened` moves the cursor there and returns that entry. Back while
    /// `active` is not the current entry's tab (e.g. no tab open) tries the current entry itself first.
    pub fn step(
        &mut self,
        dir: NavDir,
        active: Option<&K>,
        mut try_open: impl FnMut(&NavEntry<K>) -> Reach,
    ) -> Option<NavEntry<K>> {
        if self.entries.is_empty() {
            return None;
        }
        let mut next = match dir {
            NavDir::Back if !self.on_current(active) => Some(self.cursor),
            NavDir::Back => self.cursor.checked_sub(1),
            NavDir::Forward => Some(self.cursor + 1),
        };
        // The current entry was tried and removed: there is no entry left behind.
        let mut cursor_gone = false;
        while let Some(at) = next.filter(|&at| at < self.entries.len()) {
            match try_open(&self.entries[at]) {
                Reach::Opened => {
                    if cursor_gone {
                        self.left = None;
                    } else if at != self.cursor {
                        self.left = Some(self.cursor);
                    }
                    self.cursor = at;
                    return Some(self.entries[at].clone());
                }
                Reach::Failed => {
                    next = match dir {
                        NavDir::Back => at.checked_sub(1),
                        NavDir::Forward => Some(at + 1),
                    }
                }
                Reach::Gone => {
                    cursor_gone |= at == self.cursor;
                    self.remove(at);
                    next = match dir {
                        NavDir::Back => at.checked_sub(1),
                        NavDir::Forward => Some(at),
                    }
                }
            }
        }
        None
    }

    /// Rewrites every entry's key with `f` (an asset that was renamed).
    pub fn rewrite(&mut self, mut f: impl FnMut(&mut K)) {
        for entry in &mut self.entries {
            f(&mut entry.tab);
        }
    }

    fn on_current(&self, active: Option<&K>) -> bool {
        self.current().map(|e| &e.tab) == active
    }

    /// Truncates the forward part, pushes `entry` as the current one and applies the cap.
    fn push(&mut self, entry: NavEntry<K>) {
        if !self.entries.is_empty() {
            self.entries.truncate(self.cursor + 1);
        }
        self.left = self.left.filter(|&i| i < self.entries.len());
        self.entries.push(entry);
        self.cursor = self.entries.len() - 1;
        let over = self.entries.len().saturating_sub(NAV_CAP);
        if over > 0 {
            self.entries.drain(..over);
            self.cursor -= over;
            self.left = self.left.and_then(|i| i.checked_sub(over));
        }
    }

    /// Removes entry `at`, keeping the cursor and `left` on the same logical entries.
    fn remove(&mut self, at: usize) {
        self.entries.remove(at);
        if at < self.cursor {
            self.cursor -= 1;
        } else if at == self.cursor {
            self.cursor = at.min(self.entries.len().saturating_sub(1));
        }
        self.left = match self.left {
            Some(i) if i == at => None,
            Some(i) if i > at => Some(i - 1),
            left => left,
        };
    }
}

/// Whether moving the camera from `a` to `b` (the new view) is a jump (decision 0041).
fn is_jump(a: NavView, b: NavView, min_side: f32) -> bool {
    (a.centre - b.centre).length() * b.zoom > NAV_JUMP_SCREEN * min_side
        || (a.zoom / b.zoom).max(b.zoom / a.zoom) >= NAV_JUMP_ZOOM
}

impl NavHistory<TabKey> {
    /// The tab `from` is now `to` (an asset was renamed, design §14.4): the entries naming it, or a view derived from
    /// it ([`TabKey::renamed`]), name the new key. Core calls it on every rename, for every domain.
    pub fn rename(&mut self, from: &TabKey, to: &TabKey) {
        self.rewrite(|key| {
            if let Some(new) = key.renamed(from, to) {
                *key = new;
            }
        });
    }
}
