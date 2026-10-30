//! Curve tabs (decision 0015).

use super::*;

pub(super) fn pulse() -> AssetId {
    AssetId::new(AssetType::Curve, "common/pulse")
}

pub(super) fn points(e: &Editor) -> Vec<(f32, f32)> {
    e.curve_doc().unwrap().curve.points.clone()
}

#[test]
fn tab_labels_are_short_unless_shared() {
    let (_dir, mut e) = project();
    e.regenerate();
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(Action::OpenAsset(rock.to_ref()));
    e.apply(Action::OpenAsset(pulse().to_ref()));
    assert_eq!(crate::tab::labels(&e.tabs), ["rock_wet", "pulse"]);
    // Generated files are named `mat_*` (decision 0020): no clash with the graph tab.
    e.apply(MaterialAction::ShowGenerated(rock.clone(), WESL));
    assert_eq!(crate::tab::labels(&e.tabs), ["rock_wet", "pulse", "mat_rock_wet"]);
    e.apply(MaterialAction::ShowGenerated(rock, RUST));
    assert_eq!(
        crate::tab::labels(&e.tabs),
        [
            "rock_wet",
            "pulse",
            "assets/quillshade/material/autogen/dir_env/mat_rock_wet.wesl",
            "crates/quillshade/src/material/materials/dir_env/mat_rock_wet.rs"
        ]
    );
}

#[test]
fn curve_tab_edits_undo_clipboard_and_save() {
    let (dir, mut e) = project();
    e.apply(Action::OpenAsset(pulse().to_ref()));
    assert!(e.doc().is_none(), "graph commands must not reach a curve tab");
    assert_eq!(e.tab().unwrap().title(), "quillshade/material/common/pulse.curve");
    let n = points(&e).len();

    e.apply(Action::from(CurveEdit::new(CurveAction::Add((0.6, 0.9)))));
    assert_eq!(points(&e).len(), n + 1);
    assert!(e.is_dirty());
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(points(&e).len(), n);
    assert!(!e.is_dirty());
    e.apply(Action::Tab(TabAction::Redo));

    // Copy the new point, paste it twice, then select all and delete everything.
    e.apply(Action::Tab(TabAction::Copy));
    e.apply(Action::Tab(TabAction::Paste(None)));
    assert_eq!(points(&e).len(), n + 2);
    assert_eq!(e.curve_doc().unwrap().selection.len(), 1, "the pasted point is selected");
    e.apply(Action::Tab(TabAction::SelectAll));
    assert_eq!(e.curve_doc().unwrap().selection.len(), n + 2);
    e.apply(Action::Tab(TabAction::DeleteSelection));
    assert!(points(&e).is_empty());
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(points(&e).len(), n + 2);

    // Properties edit the selected point; the preview switch does not touch the file.
    e.apply(Action::from(CurveEdit::new(CurveAction::Select([0].into()))));
    let target = CurveKey::new(CurveProp::Point(0));
    e.apply(set_component(target.clone(), 1, 0.5, true));
    assert_eq!(points(&e)[0], (0.0, 0.5));
    let saved_rev = e.curve_doc().unwrap().saved.clone();
    e.apply(set_value(CurveKey::new(CurveProp::Preview), Value::Str("Simple".into()), false));
    assert_eq!(e.curve_doc().unwrap().preview, Interp::Simple);
    assert_eq!(e.curve_doc().unwrap().saved, saved_rev);

    e.apply(Action::Save);
    assert!(!e.is_dirty());
    let wesl =
        std::fs::read_to_string(dir.path().join("assets/quillshade/material/autogen/curves/dir_common/crv_pulse.wesl"))
            .unwrap();
    assert!(wesl.contains("vec3<f32>(0.0, 0.5, "), "{wesl}");
    let on_disk = assets::load_curve(e.project.as_ref().unwrap(), &pulse()).unwrap();
    assert_eq!(on_disk.points, points(&e));
}

#[test]
fn nudges_merge_until_the_selection_changes() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(pulse().to_ref()));
    let start = points(&e);
    e.apply(Action::from(CurveEdit::new(CurveAction::Select([1].into()))));
    e.apply(Action::from(CurveEdit::new(CurveAction::Nudge((0.0, -0.1)))));
    e.apply(Action::from(CurveEdit::new(CurveAction::Nudge((0.0, -0.1)))));
    e.apply(Action::from(CurveEdit::new(CurveAction::Select([2].into()))));
    e.apply(Action::from(CurveEdit::new(CurveAction::Nudge((0.0, 0.1)))));
    e.apply(Action::Tab(TabAction::Undo));
    assert!((points(&e)[1].1 - (start[1].1 - 0.2)).abs() < 1e-6);
    assert_eq!(points(&e)[2], start[2]);
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(points(&e), start);
}

pub(super) fn curve_range(e: &Editor) -> (f32, f32) {
    let c = &e.curve_doc().unwrap().curve;
    (c.min, c.max)
}

pub(super) fn set_range(e: &mut Editor, comp: usize, value: f32, is_final: bool) {
    e.apply(set_component(CurveKey::new(CurveProp::Range), comp, value, is_final));
}

#[test]
fn curve_range_edits_are_undoable_and_checked() {
    let (dir, mut e) = project();
    e.apply(Action::OpenAsset(pulse().to_ref()));
    let start = points(&e);
    assert_eq!(shown(&e, &CurveKey::new(CurveProp::Range)), Some(Value::Float2([0.0, 1.0])));

    // A Max scrub is one undo step; Min typed is another; the points stay normalized.
    for v in [2.0, 3.0, 4.0] {
        set_range(&mut e, 1, v, false);
    }
    set_range(&mut e, 1, 4.0, true);
    set_range(&mut e, 0, -2.0, true);
    assert_eq!(curve_range(&e), (-2.0, 4.0));
    assert_eq!(points(&e), start);
    assert!(e.is_dirty());

    // `min >= max` is refused with a warning and changes nothing.
    let warnings = e.messages.iter().filter(|m| m.level == Level::Warning).count();
    set_range(&mut e, 0, 4.0, true);
    set_range(&mut e, 1, -3.0, true);
    assert_eq!(curve_range(&e), (-2.0, 4.0));
    let new: Vec<&Message> = e.messages.iter().filter(|m| m.level == Level::Warning).skip(warnings).collect();
    assert_eq!(new.len(), 2, "{new:?}");
    assert!(new.iter().all(|m| m.text.contains("min")), "{new:?}");

    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(curve_range(&e), (0.0, 4.0));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(curve_range(&e), (0.0, 1.0));
    e.apply(Action::Tab(TabAction::Redo));
    e.apply(Action::Tab(TabAction::Redo));
    assert_eq!(curve_range(&e), (-2.0, 4.0));

    // The selected point's Value field shows and edits real units; T stays normalized.
    e.apply(Action::from(CurveEdit::new(CurveAction::Select([1].into()))));
    let target = CurveKey::new(CurveProp::Point(1));
    assert_eq!(shown(&e, &target), Some(Value::Float2([0.25, 4.0])));
    e.apply(set_component(target.clone(), 1, 1.0, true));
    assert_eq!(points(&e)[1], (0.25, 0.5));
    e.apply(set_component(target.clone(), 0, 0.2, true));
    assert_eq!(points(&e)[1], (0.2, 0.5), "a T edit keeps the stored value exactly");
    // A typed value outside the range is clamped to it, like a dragged point.
    e.apply(set_component(target, 1, 9.0, true));
    assert_eq!(points(&e)[1], (0.2, 1.0));

    e.apply(Action::Save);
    let on_disk = assets::load_curve(e.project.as_ref().unwrap(), &pulse()).unwrap();
    assert_eq!((on_disk.min, on_disk.max), (-2.0, 4.0));
    let wesl =
        std::fs::read_to_string(dir.path().join("assets/quillshade/material/autogen/curves/dir_common/crv_pulse.wesl"))
            .unwrap();
    assert!(wesl.contains("vec3<f32>(0.2, 4.0, "), "{wesl}");
}

#[test]
fn new_and_renamed_curves_follow_their_users() {
    let (dir, mut e) = project();
    e.apply(new_asset("curve", 0));
    e.apply(Action::Submit("ramps/fade".into()));
    let fade = AssetId::new(AssetType::Curve, "ramps/fade");
    assert_eq!(e.curve_index(&fade), e.active);
    assert!(dir.path().join("assets/quillshade/material/autogen/curves/dir_ramps/crv_fade.wesl").is_file());
    assert!(e.curve_paths().contains(&"ramps/fade".to_string()));

    // A material using `common/pulse` is open; renaming the curve updates the node, the file and the tab.
    let bands = AssetId::new(AssetType::Material, "fx/curve_bands");
    e.apply(Action::OpenAsset(bands.to_ref()));
    e.apply(Action::OpenAsset(pulse().to_ref()));
    e.apply(Action::RenameDialog(pulse().to_ref()));
    e.apply(Action::Submit("ramps/pulse".into()));
    let moved = AssetId::new(AssetType::Curve, "ramps/pulse");
    assert!(e.curve_index(&moved).is_some());
    let d = e.tabs[e.doc_index(&bands).unwrap()].get::<Doc>().unwrap();
    assert!(!d.is_dirty());
    let refs = shader_graph_core::registry::curve_refs(&d.asset, &e.domains.material().registry);
    assert!(refs.iter().any(|r| r.2 == "ramps/pulse"), "{refs:?}");
    let node = refs.iter().find(|r| r.2 == "ramps/pulse").unwrap().0.clone();
    assert_eq!(
        crate::doc::node_title(&e.domains.material().registry, &e.domains.material().functions, &d.asset, &node),
        "pulse (Sample Curve)"
    );
    e.validate();
    assert!(!e.all_diagnostics().iter().any(Diagnostic::is_error), "{:?}", e.all_diagnostics());
}

#[test]
fn curve_tabs_are_restored_on_next_launch() {
    let (dir, mut e) = project();
    let config = dir.path().join("config/config.ron");
    e.config_path = Some(config.clone());
    e.apply(Action::OpenAsset(pulse().to_ref()));
    e.apply(Action::Quit);
    let mut next = crate::test_editor().with_config(UserConfig::load(&config), Some(config));
    next.open_project(dir.path()).unwrap();
    assert_eq!(next.curve_doc().map(|c| &c.aref), Some(&pulse().to_ref()));
}

#[test]
fn curve_review_regressions() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(pulse().to_ref()));
    let start = points(&e);

    // Typing a T that moves the point past a neighbour rebinds the fields to its new index.
    e.apply(Action::from(CurveEdit::new(CurveAction::Select([1].into()))));
    let props = e.ui.props;
    e.apply(set_component(CurveKey::new(CurveProp::Point(1)), 0, 0.6, true));
    assert_eq!(e.curve_doc().unwrap().selection, [2].into());
    assert!(e.ui.props > props);
    assert_eq!(points(&e)[2], (0.6, 1.0));
    e.apply(set_component(CurveKey::new(CurveProp::Point(2)), 1, 0.9, true));
    assert_eq!(points(&e)[2], (0.6, 0.9));
    // Two typed edits, two undo steps.
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(points(&e)[2], (0.6, 1.0));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(points(&e), start);

    // A clamped (no-op) drag does not glue the next real drag to the previous step.
    e.apply(Action::from(CurveEdit::new(CurveAction::Add((0.9, 0.5)))));
    let added = points(&e);
    let last = added.len() - 1;
    let drag = |e: &mut Editor, delta| {
        let (points, selection) = CurveDoc::moved(&added, &[last].into(), delta);
        e.apply(Action::from(CurveEdit::new(CurveAction::SetPoints { points, selection, merge: true })));
    };
    drag(&mut e, (0.5, 0.5));
    drag(&mut e, (0.0, -0.5));
    e.apply(Action::Tab(TabAction::CommitInline));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(points(&e), added);

    // Paste and Ctrl+D place copies beside the originals; Ctrl+D keeps the clipboard.
    e.apply(Action::from(CurveEdit::new(CurveAction::Select([2].into()))));
    e.apply(Action::Tab(TabAction::Copy));
    let clipboard = |e: &Editor| e.clipboards.get::<CurveClipboard>().map(|c| c.points.clone());
    let copied = clipboard(&e);
    e.apply(Action::Tab(TabAction::Paste(None)));
    e.apply(Action::Tab(TabAction::Duplicate));
    assert_eq!(clipboard(&e), copied);
    let p = points(&e);
    assert_eq!(p.len(), added.len() + 2);
    let (t, v) = added[2];
    assert!(
        p.iter()
            .filter(|q| q.1 == v)
            .all(|q| q.0 == t || (q.0 - t - 0.05).abs() < 1e-6 || (q.0 - t - 0.1).abs() < 1e-6)
    );
    assert_eq!(p.iter().filter(|q| **q == (t, v)).count(), 1, "no copy on top of the original");
}
