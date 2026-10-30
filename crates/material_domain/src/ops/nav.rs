//! The navigation history (decision 0041) over core's tab keys: the history itself is core's
//! ([`editor_core::nav`]), and so is renaming the keys of a renamed asset, its generated files' (`…#wesl`) included
//! (design §14.4).

use editor_core::tab::TabKey;

pub use editor_core::nav::{NAV_CAP, NAV_JUMP_SCREEN, NAV_JUMP_ZOOM, NavDir, NavView, Reach};

/// The Back / Forward list of [`TabKey`]s.
pub type NavHistory = editor_core::nav::NavHistory<TabKey>;
/// One place in the history.
pub type NavEntry = editor_core::nav::NavEntry<TabKey>;

#[cfg(test)]
mod tests {
    use bevy::math::Vec2;
    use shader_project::{AssetId, AssetType};

    use super::*;
    use crate::tab::{MaterialKey, WESL};

    fn k(p: &str) -> TabKey {
        mk(MaterialKey::Asset(AssetId::new(AssetType::Material, p)))
    }

    fn mk(key: MaterialKey) -> TabKey {
        crate::tab::tab_key(&key)
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
        h.visit(mk(MaterialKey::Asset(x.clone())));
        h.visit(mk(MaterialKey::Generated(x.clone(), WESL)));
        h.visit(mk(MaterialKey::Asset(y.clone())));
        // Core's rename (design §14.4) also rewrites the generated file's key.
        h.rename(&mk(MaterialKey::Asset(x.clone())), &mk(MaterialKey::Asset(z.clone())));
        assert_eq!(
            tabs(&h),
            vec![mk(MaterialKey::Asset(z.clone())), mk(MaterialKey::Generated(z, WESL)), mk(MaterialKey::Asset(y))]
        );
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
