//! Navigation history (Back / Forward): one flat list of visited tabs and graph camera views with a cursor, like a
//! browser. New entries only on a tab change or a large camera jump; small moves amend the current entry. In memory
//! only, decision 0041.

use bevy::math::Vec2;
use shader_project::AssetId;

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
pub struct NavEntry {
    pub tab: TabKey,
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

/// The Back / Forward list with its cursor.
#[derive(Debug, Default, Clone)]
pub struct NavHistory {
    entries: Vec<NavEntry>,
    /// The current entry (meaningless while `entries` is empty).
    cursor: usize,
    /// The entry the cursor last moved away from (`visit` / `step`), for [`NavHistory::amend_last_of`].
    left: Option<usize>,
}

impl NavHistory {
    pub fn current(&self) -> Option<&NavEntry> {
        self.entries.get(self.cursor)
    }

    pub fn entries(&self) -> &[NavEntry] {
        &self.entries
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The active tab became `tab`: no-op if the current entry is that tab; else truncate forward, push
    /// `{tab, None}`, cap.
    pub fn visit(&mut self, tab: TabKey) {
        if self.current().is_some_and(|e| e.tab == tab) {
            return;
        }
        self.left = (!self.entries.is_empty()).then_some(self.cursor);
        self.push(NavEntry { tab, view: None });
    }

    /// A settled camera of `tab` (`min_side`: the canvas's smaller side, logical px). Ignored unless the current
    /// entry is `tab`. Sets a missing view, pushes `{tab, Some(view)}` on a jump (truncating forward), else amends.
    pub fn note_view(&mut self, tab: &TabKey, view: NavView, min_side: f32) {
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
    pub fn amend_last_of(&mut self, tab: &TabKey, view: NavView) {
        if let Some(entry) = self.left.and_then(|i| self.entries.get_mut(i)).filter(|e| e.tab == *tab) {
            entry.view = Some(view);
        }
    }

    /// Whether Back has an entry to try (validity is not checked). With `active` not the current entry's tab
    /// (e.g. no tab open) that is the current entry itself.
    pub fn can_back(&self, active: Option<&TabKey>) -> bool {
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
        active: Option<&TabKey>,
        mut try_open: impl FnMut(&NavEntry) -> Reach,
    ) -> Option<NavEntry> {
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

    /// Rewrites `Asset(from)` / `Generated(from, _)` keys to `to`.
    pub fn rename(&mut self, from: &AssetId, to: &AssetId) {
        for entry in &mut self.entries {
            if let TabKey::Asset(id) | TabKey::Generated(id, _) = &mut entry.tab
                && id == from
            {
                *id = to.clone();
            }
        }
    }

    fn on_current(&self, active: Option<&TabKey>) -> bool {
        self.current().map(|e| &e.tab) == active
    }

    /// Truncates the forward part, pushes `entry` as the current one and applies the cap.
    fn push(&mut self, entry: NavEntry) {
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

#[cfg(test)]
mod tests {
    use shader_project::AssetType;

    use super::*;
    use crate::tab::Lang;

    fn k(p: &str) -> TabKey {
        TabKey::Asset(AssetId::new(AssetType::Material, p))
    }

    fn v(x: f32, zoom: f32) -> NavView {
        NavView { centre: Vec2::new(x, 0.0), zoom }
    }

    fn tabs(h: &NavHistory) -> Vec<TabKey> {
        h.entries().iter().map(|e| e.tab.clone()).collect()
    }

    fn opened(_: &NavEntry) -> Reach {
        Reach::Opened
    }

    #[test]
    fn visit_pushes_and_dedups() {
        let mut h = NavHistory::default();
        h.visit(k("a"));
        h.visit(k("a"));
        h.visit(k("b"));
        assert_eq!(h.entries().len(), 2);
        assert_eq!(h.current().map(|e| &e.tab), Some(&k("b")));
    }

    #[test]
    fn back_then_visit_truncates_forward() {
        let mut h = NavHistory::default();
        for p in ["a", "b", "c"] {
            h.visit(k(p));
        }
        let got = h.step(NavDir::Back, Some(&k("c")), opened);
        assert_eq!(got.map(|e| e.tab), Some(k("b")));
        h.visit(k("d"));
        assert_eq!(tabs(&h), vec![k("a"), k("b"), k("d")]);
        assert!(!h.can_forward());
    }

    #[test]
    fn note_view_amends_small_and_pushes_jump() {
        let mut h = NavHistory::default();
        h.visit(k("a"));
        h.note_view(&k("a"), v(0.0, 1.0), 600.0);
        assert_eq!(h.entries().len(), 1);
        assert_eq!(h.current().and_then(|e| e.view), Some(v(0.0, 1.0)));
        h.note_view(&k("a"), v(100.0, 1.0), 600.0);
        assert_eq!(h.entries().len(), 1);
        assert_eq!(h.current().and_then(|e| e.view), Some(v(100.0, 1.0)));
        // 400 px > 300 px (half of the smaller side).
        h.note_view(&k("a"), v(500.0, 1.0), 600.0);
        assert_eq!(h.entries().len(), 2);
        // Zoom x2.
        h.note_view(&k("a"), v(500.0, 2.0), 600.0);
        assert_eq!(h.entries().len(), 3);
        // 2 / 1.2 < 2.
        h.note_view(&k("a"), v(500.0, 1.2), 600.0);
        assert_eq!(h.entries().len(), 3);
        assert_eq!(h.current().and_then(|e| e.view), Some(v(500.0, 1.2)));
    }

    #[test]
    fn note_view_ignores_other_tab() {
        let mut h = NavHistory::default();
        h.visit(k("a"));
        let before = h.entries().to_vec();
        h.note_view(&k("b"), v(10.0, 1.0), 600.0);
        assert_eq!(h.entries(), &before[..]);
    }

    #[test]
    fn step_drops_gone_keeps_failed() {
        let mut h = NavHistory::default();
        for p in ["a", "b", "c", "d"] {
            h.visit(k(p));
        }
        let got = h.step(NavDir::Back, Some(&k("d")), |e| {
            if e.tab == k("c") {
                Reach::Gone
            } else if e.tab == k("b") {
                Reach::Failed
            } else {
                Reach::Opened
            }
        });
        assert_eq!(got.map(|e| e.tab), Some(k("a")));
        assert_eq!(tabs(&h), vec![k("a"), k("b"), k("d")]);
        let mut tried = Vec::new();
        let got = h.step(NavDir::Forward, Some(&k("a")), |e| {
            tried.push(e.tab.clone());
            Reach::Opened
        });
        assert_eq!(tried, vec![k("b")]);
        assert_eq!(got.map(|e| e.tab), Some(k("b")));
    }

    #[test]
    fn step_nothing_valid_returns_none_and_keeps_cursor() {
        let mut h = NavHistory::default();
        h.visit(k("a"));
        h.visit(k("b"));
        let got = h.step(NavDir::Back, Some(&k("b")), |_| Reach::Failed);
        assert_eq!(got, None);
        assert_eq!(h.current().map(|e| &e.tab), Some(&k("b")));
        assert_eq!(h.entries().len(), 2);
    }

    #[test]
    fn back_with_no_active_tab_reopens_current() {
        let mut h = NavHistory::default();
        h.visit(k("a"));
        h.visit(k("b"));
        let got = h.step(NavDir::Back, None, opened);
        assert_eq!(got.map(|e| e.tab), Some(k("b")));
        let got = h.step(NavDir::Back, Some(&k("b")), opened);
        assert_eq!(got.map(|e| e.tab), Some(k("a")));
    }

    #[test]
    fn amend_last_of_sets_previous_entry() {
        let mut h = NavHistory::default();
        h.visit(k("a"));
        h.visit(k("b"));
        h.amend_last_of(&k("a"), v(7.0, 1.0));
        assert_eq!(h.entries()[0].view, Some(v(7.0, 1.0)));
        assert_eq!(h.entries().len(), 2);
    }

    #[test]
    fn amend_last_of_after_back_amends_left_entry() {
        let mut h = NavHistory::default();
        for p in ["a", "b", "a", "b"] {
            h.visit(k(p));
        }
        h.step(NavDir::Back, Some(&k("b")), opened);
        assert_eq!(h.current().map(|e| &e.tab), Some(&k("a")));
        h.amend_last_of(&k("b"), v(9.0, 1.0));
        assert_eq!(h.entries()[3].view, Some(v(9.0, 1.0)));
        assert_eq!(h.entries()[1].view, None);
    }

    #[test]
    fn amend_last_of_other_tab_does_nothing() {
        let mut h = NavHistory::default();
        h.visit(k("a"));
        h.visit(k("b"));
        h.amend_last_of(&k("c"), v(7.0, 1.0));
        assert!(h.entries().iter().all(|e| e.view.is_none()));
    }

    #[test]
    fn amend_last_of_follows_removed_entries() {
        let mut h = NavHistory::default();
        for p in ["a", "b", "c", "d"] {
            h.visit(k(p));
        }
        // Back from d drops c (gone) and opens b: the entry left is d, now at index 2.
        h.step(NavDir::Back, Some(&k("d")), |e| if e.tab == k("c") { Reach::Gone } else { Reach::Opened });
        h.amend_last_of(&k("d"), v(3.0, 1.0));
        assert_eq!(h.entries()[2].view, Some(v(3.0, 1.0)));
    }

    #[test]
    fn rename_rewrites_asset_and_generated() {
        let x = AssetId::new(AssetType::Material, "x");
        let y = AssetId::new(AssetType::Material, "y");
        let z = AssetId::new(AssetType::Material, "z");
        let mut h = NavHistory::default();
        h.visit(TabKey::Asset(x.clone()));
        h.visit(TabKey::Generated(x.clone(), Lang::Wesl));
        h.visit(TabKey::Asset(y.clone()));
        h.rename(&x, &z);
        assert_eq!(tabs(&h), vec![TabKey::Asset(z.clone()), TabKey::Generated(z, Lang::Wesl), TabKey::Asset(y)]);
    }

    #[test]
    fn cap_drops_oldest() {
        let mut h = NavHistory::default();
        for i in 0..=NAV_CAP {
            h.visit(k(&format!("t{i}")));
        }
        assert_eq!(h.entries().len(), NAV_CAP);
        assert_eq!(h.entries()[0].tab, k("t1"));
        assert_eq!(h.current().map(|e| &e.tab), Some(&k(&format!("t{NAV_CAP}"))));
        // The entry left by the last visit moved with the trim.
        h.amend_last_of(&k(&format!("t{}", NAV_CAP - 1)), v(1.0, 1.0));
        assert_eq!(h.entries()[NAV_CAP - 2].view, Some(v(1.0, 1.0)));
    }

    #[test]
    fn can_back_forward_ends() {
        let mut h = NavHistory::default();
        assert!(!h.can_back(None));
        assert!(!h.can_forward());
        h.visit(k("a"));
        h.visit(k("b"));
        assert!(h.can_back(Some(&k("b"))));
        assert!(!h.can_forward());
        h.step(NavDir::Back, Some(&k("b")), opened);
        assert!(h.can_forward());
        assert!(!h.can_back(Some(&k("a"))));
        // No tab open: Back reopens the current entry.
        assert!(h.can_back(None));
    }

    #[test]
    fn clear_empties() {
        let mut h = NavHistory::default();
        h.visit(k("a"));
        h.clear();
        assert!(h.entries().is_empty());
        assert_eq!(h.current(), None);
    }
}
